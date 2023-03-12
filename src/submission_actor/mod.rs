//! The central actor that does all HTTP requests.

mod api;

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use mpd_client::responses::Song;
use once_cell::sync::OnceCell;
use reqwest::{
    header::{self, HeaderMap, HeaderValue},
    Client, StatusCode,
};
use tokio::{
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
    time::sleep,
};
use tracing::{debug, error, trace, warn};

use self::api::SerializedSubmission;
use crate::config::Configuration;

/// API sub-path to which listen records are submitted.
const LISTENBRAINZ_SUBMISSION_PATH: &str = "/1/submit-listens";

fn submission_url(config: &Configuration) -> &'static str {
    static URL: OnceCell<String> = OnceCell::new();

    URL.get_or_init(|| {
        let base = if config.submission.api_url.ends_with('/') {
            &config.submission.api_url[..config.submission.api_url.len() - 1]
        } else {
            &config.submission.api_url
        };

        format!("{base}{LISTENBRAINZ_SUBMISSION_PATH}")
    })
}

/// Central actor that handles HTTP requests.
#[derive(Clone)]
pub struct SubmissionActor {
    tx: UnboundedSender<ActorRequest>,
}

impl SubmissionActor {
    /// Start the submission actor.
    pub async fn start(configuration: Configuration) -> Result<SubmissionActor> {
        let http_client = build_http_client(&configuration);

        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(run(http_client, configuration, rx));

        Ok(SubmissionActor { tx })
    }

    /// Submit a "Now Playing" event.
    pub fn now_playing(&self, song: Song) {
        self.tx
            .send(ActorRequest::NowPlaying { song })
            .expect("actor gone");
    }

    /// Submit a completed listen.
    pub fn listen(&self, song: Song, timestamp: u64) {
        self.tx
            .send(ActorRequest::Listen { song, timestamp })
            .expect("actor gone");
    }
}

fn build_http_client(configuration: &Configuration) -> Client {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Token {}", configuration.submission.token.value()))
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

async fn run(
    http_client: Client,
    config: Configuration,
    mut requests: UnboundedReceiver<ActorRequest>,
) {
    while let Some(request) = requests.recv().await {
        match request {
            ActorRequest::NowPlaying { song } => {
                let Some(submission) = api::prepare_playing_now(&config, song) else { continue; };
                if let Err(e) = submit(&http_client, &config, &submission)
                    .await
                    .context("Submission of \"Playing Now\" notification failed")
                {
                    error!("{e:#}");
                }
            }
            ActorRequest::Listen { song, timestamp } => {
                let Some(listen) = api::serialize_single_listen(&config, song, timestamp) else { continue; };
                let submission = api::prepare_completed_listens(&listen);

                if let Err(e) = submit(&http_client, &config, &submission)
                    .await
                    .context("Submission of completed Listen failed")
                {
                    error!("{e:#}");
                }
            }
        }
    }
}

async fn submit(
    http_client: &Client,
    config: &Configuration,
    payload: &SerializedSubmission,
) -> Result<()> {
    loop {
        match do_submit(http_client, submission_url(config), payload).await {
            Ok(()) => {
                debug!("submission accepted");
                break Ok(());
            }
            Err(SubmitError::RateLimit { retry_after }) => {
                warn!(?retry_after, "hit API rate limit");
                sleep(retry_after).await;
            }
            Err(SubmitError::Error(e)) => break Err(e),
        }
    }
}

enum SubmitError {
    RateLimit { retry_after: Duration },
    Error(anyhow::Error),
}

impl From<anyhow::Error> for SubmitError {
    fn from(e: anyhow::Error) -> Self {
        SubmitError::Error(e)
    }
}

async fn do_submit(
    http_client: &Client,
    url: &str,
    submission: &SerializedSubmission,
) -> Result<(), SubmitError> {
    let response = http_client
        .post(url)
        .json(submission)
        .send()
        .await
        .context("Error sending ListenBrainz submission request")?;

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
        .context("Error reading response body")?;

    trace!(?status_code, ?response_body);

    match status_code {
        StatusCode::OK => Ok(()),
        StatusCode::UNAUTHORIZED => {
            Err(anyhow!("Invalid ListenBrainz token (please update your configuration)").into())
        }
        StatusCode::TOO_MANY_REQUESTS => Err(SubmitError::RateLimit { retry_after }),
        other_status => Err(anyhow!("Unexpected status code ({:?})", other_status).into()),
    }
}
