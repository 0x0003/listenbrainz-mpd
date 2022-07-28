//! The central actor that does all HTTP requests.

mod api;

use std::time::Duration;

use anyhow::{bail, Context, Result};
use mpd_client::commands::responses::Song;
use reqwest::{
    header::{self, HeaderMap, HeaderValue},
    Client, StatusCode,
};
use tokio::{
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
    time::sleep,
};
use tracing::{debug, error, info_span, trace, warn, Instrument};

use self::api::ValidateToken;
use crate::config::Configuration;

/// API URL to which listen records are submitted.
const LISTENBRAINZ_SUBMISSION_URL: &str = "/1/submit-listens";

/// API URL used to check if the login token is valid.
const LISTENBRAINZ_TOKEN_CHECK_URL: &str = "/1/validate-token";

/// Build a URL from the given base and path segment.
fn build_url(base: &str, url: &str) -> String {
    let url = if base.ends_with('/') && url.starts_with('/') {
        // Overlapping slashes
        &url[1..]
    } else {
        url
    };

    let mut out = String::with_capacity(base.len() + url.len());
    out.push_str(base);
    out.push_str(url);

    out
}

/// Central actor that handles HTTP requests.
#[derive(Clone)]
pub(crate) struct SubmissionActor {
    tx: UnboundedSender<ActorRequest>,
}

impl SubmissionActor {
    /// Start the submission actor.
    pub(crate) async fn start(configuration: Configuration) -> Result<SubmissionActor> {
        let http_client = build_http_client(&configuration);

        // Check if the configured login token is actually valid
        debug!("checking login token");
        let token_valid = http_client
            .get(build_url(
                &configuration.api_url,
                LISTENBRAINZ_TOKEN_CHECK_URL,
            ))
            .send()
            .await
            .context("Failed to check ListenBrainz token")?
            .json::<ValidateToken>()
            .await
            .context("Failed to check ListenBrainz token")?;

        if token_valid.valid {
            debug!(username = %token_valid.user_name, "user token is valid");
        } else {
            bail!("The ListenBrainz user token is invalid");
        }

        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(run(http_client, configuration, rx));

        Ok(SubmissionActor { tx })
    }

    /// Submit a "Now Playing" event.
    pub(crate) fn now_playing(&self, song: Song) {
        self.tx
            .send(ActorRequest::NowPlaying { song })
            .expect("actor gone");
    }

    /// Submit a completed listen.
    pub(crate) fn listen(&self, song: Song, timestamp: u64) {
        self.tx
            .send(ActorRequest::Listen { song, timestamp })
            .expect("actor gone");
    }
}

fn build_http_client(configuration: &Configuration) -> Client {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Token {}", configuration.token))
            .expect("failed to create Authorization header"),
    );

    reqwest::ClientBuilder::new()
        .default_headers(headers)
        .build()
        .expect("failed to create client")
}

#[derive(Debug)]
enum ActorRequest {
    NowPlaying { song: Song },
    Listen { song: Song, timestamp: u64 },
}

impl ActorRequest {
    fn song(&self) -> &str {
        match self {
            ActorRequest::NowPlaying { song } => &song.url,
            ActorRequest::Listen { song, .. } => &song.url,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            ActorRequest::NowPlaying { .. } => "now_playing",
            ActorRequest::Listen { .. } => "listen",
        }
    }

    fn into_submission(self, config: &Configuration) -> Option<api::Submission> {
        match self {
            ActorRequest::NowPlaying { song } => api::Submission::playing_now(config, song),
            ActorRequest::Listen { song, timestamp } => {
                api::Submission::listen(config, song, timestamp)
            }
        }
    }
}

async fn run(
    http_client: Client,
    configuration: Configuration,
    mut requests: UnboundedReceiver<ActorRequest>,
) {
    while let Some(request) = requests.recv().await {
        let span = info_span!("submission", song = %request.song(), kind = %request.kind());

        let submission = if let Some(s) = span.in_scope(|| request.into_submission(&configuration))
        {
            s
        } else {
            continue;
        };

        if let Err(error) = submit(http_client.clone(), &configuration, &submission)
            .instrument(span.clone())
            .await
        {
            error!(parent: &span, ?error, "failed ListenBrainz submission");
        }
    }
}

async fn submit(
    http_client: Client,
    configuration: &Configuration,
    payload: &api::Submission,
) -> Result<()> {
    // Inner loop to allow retrying the request on rate limit
    loop {
        let response = http_client
            .post(build_url(
                &configuration.api_url,
                LISTENBRAINZ_SUBMISSION_URL,
            ))
            .json(payload)
            .send()
            .await
            .context("error sending ListenBrainz submission request")?;

        let status_code = response.status();
        let retry_after = response
            .headers()
            .get("X-RateLimit-Reset-In")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .map_or(Duration::from_secs(10), Duration::from_secs);
        let response_body = response
            .bytes()
            .await
            .context("error reading response body")?;
        trace!(?status_code, ?response_body);

        match status_code {
            StatusCode::OK => break,
            StatusCode::TOO_MANY_REQUESTS => {
                warn!(?retry_after, "hit ListenBrainz API rate limit");
                sleep(retry_after).await;
            }
            other_status => bail!("unexpected status code ({:?})", other_status),
        }
    }

    debug!("submission accepted");

    Ok(())
}
