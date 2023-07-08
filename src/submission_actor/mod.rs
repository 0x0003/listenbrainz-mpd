//! The central actor that does all HTTP requests.

mod api;

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use bytes::Bytes;
use mpd_client::responses::Song;
use once_cell::sync::OnceCell;
use reqwest::{
    header::{self, HeaderMap, HeaderValue},
    Client, Method, Request, RequestBuilder, StatusCode,
};
use tokio::{
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
    time::sleep,
};
use tracing::{debug, error, trace, warn};

use self::api::JsonBody;
use crate::{cache_actor::CacheActor, config::Configuration};

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
    pub async fn start(
        configuration: Configuration,
        cache_actor: CacheActor,
    ) -> Result<SubmissionActor> {
        let http_client = build_http_client(&configuration);

        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(run(http_client, configuration, cache_actor, rx));

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
    headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));

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
    cache_actor: CacheActor,
    mut requests: UnboundedReceiver<ActorRequest>,
) {
    while let Some(request) = requests.recv().await {
        match request {
            ActorRequest::NowPlaying { song } => {
                let Some(body) = api::prepare_playing_now(&config, song) else {
                    continue;
                };
                if let Err(e) = submit(&http_client, &config, body)
                    .await
                    .context("Submission of \"Playing Now\" notification failed")
                {
                    error!("{e:#}");
                }
            }
            ActorRequest::Listen { song, timestamp } => {
                let Some(listen) = api::serialize_single_listen(&config, song, timestamp) else {
                    continue;
                };

                // Load possible cached listens
                let mut submissions = cache_actor.load_pending_submissions().await;
                submissions.push(listen);

                let body = api::prepare_completed_listens(&submissions);

                if let Err(e) = submit(&http_client, &config, body).await.context(
                    submission_failed_error_string(config.submission.enable_cache),
                ) {
                    error!("{e:#}");

                    // Cache the failed submission(s) for future resubmission
                    debug!(
                        count = submissions.len(),
                        "caching submissions for resubmission"
                    );
                    cache_actor.cache_submissions(submissions);
                }
            }
        }
    }

    cache_actor.shutdown();
}

fn submission_failed_error_string(enable_cache: bool) -> &'static str {
    if enable_cache {
        "Submission of completed Listen failed (will be cached for later submission)"
    } else {
        "Submission of completed Listen failed"
    }
}

async fn submit(http_client: &Client, config: &Configuration, body: JsonBody) -> Result<()> {
    do_http_request(http_client, |http_client| {
        http_client
            .post(submission_url(config))
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.clone())
            .build()
            .unwrap()
    })
    .await
    .context("Failed to send submission")?;

    Ok(())
}

async fn do_http_request<F>(http_client: &Client, mut build_request: F) -> Result<Bytes>
where
    F: FnMut(&Client) -> Request,
{
    loop {
        let request = build_request(http_client);
        trace!("sending request");
        let response = http_client
            .execute(request)
            .await
            .context("Error sending ListenBrainz request")?;

        match response.status() {
            StatusCode::OK => {
                let body = response
                    .bytes()
                    .await
                    .context("Error reading ListenBrainz response")?;
                trace!(?body, "request completed sucessfully");
                break Ok(body);
            }
            StatusCode::UNAUTHORIZED => {
                bail!("Invalid ListenBrainz token (please update your configuration)");
            }
            StatusCode::TOO_MANY_REQUESTS => {
                let retry_after = response
                    .headers()
                    .get("X-RateLimit-Reset-In")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .map_or(Duration::from_secs(10), Duration::from_secs);
                warn!(?retry_after, "hit rate limit");
                sleep(retry_after).await;
                debug!("trying again");
            }
            other => {
                bail!("Unexpected status code from ListenBrainz API ({other:?}");
            }
        }
    }
}
