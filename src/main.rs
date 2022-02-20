mod config;

use std::{
    cmp,
    net::SocketAddr,
    path::Path,
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use mpd_client::{
    commands::{
        self,
        responses::{PlayState, Song},
    },
    state_changes::StateChanges,
    Client, Subsystem,
};
use reqwest::header::{self, HeaderMap, HeaderValue};
use tokio::{
    net::{TcpStream, UnixStream},
    time::sleep,
};
use tokio_stream::StreamExt;
use tracing::{debug, error, info, trace, warn};
use tracing_subscriber::EnvFilter;

use crate::config::MpdConnection;

/// The maximum time you have to listen to a song before it will count as a listen. Set to 4
/// minutes as per the recommendations in the ListenBrainz documentation.
const MAX_REQUIRED_LISTEN_TIME: Duration = Duration::from_secs(20);

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_env("LISTENBRAINZ_MPD_LOG"))
        .init();

    let config = config::load(Path::new("./config.toml"))?;

    let http_client = http_client(&config)?;
    let (mpd_client, state_changes) = connect(&config.mpd).await?;

    run(mpd_client, state_changes, http_client).await
}

fn http_client(config: &config::Configuration) -> Result<reqwest::Client> {
    if config.token.is_empty() {
        bail!("The ListenBrainz user token is not set");
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Token {}", config.token))?,
    );

    Ok(reqwest::ClientBuilder::new()
        .default_headers(headers)
        .build()
        .unwrap())
}

async fn connect(mpd_config: &config::Mpd) -> Result<(Client, StateChanges)> {
    match &mpd_config.connection {
        MpdConnection::Tcp { ip, port } => {
            let address = SocketAddr::new(*ip, port.get());
            connect_tcp(address)
                .await
                .with_context(|| format!("failed to connect to {}", address))
        }
        MpdConnection::UnixSocket { unix } => connect_unix(unix)
            .await
            .with_context(|| format!("failed to connect via Unix socket at {}", unix.display())),
    }
}

async fn connect_tcp(address: SocketAddr) -> Result<(Client, StateChanges)> {
    debug!(?address, "connecting via TCP");
    let socket = TcpStream::connect(address).await?;
    Client::connect(socket).await.map_err(Into::into)
}

async fn connect_unix(path: &Path) -> Result<(Client, StateChanges)> {
    debug!(?path, "connecting via Unix socket");
    let socket = UnixStream::connect(path).await?;
    Client::connect(socket).await.map_err(Into::into)
}

async fn run(
    mpd_client: Client,
    mut state_changes: StateChanges,
    _http_client: reqwest::Client,
) -> Result<()> {
    // Get initial state of MPD
    let (mut state, mut current_song) = get_status_and_song(&mpd_client).await?;

    let mut current_song_listen_started = Instant::now();
    let mut current_song_listen_required = current_song
        .as_ref()
        .map_or(MAX_REQUIRED_LISTEN_TIME, required_time_for_song);

    let mut current_song_listen_finished = Box::pin(sleep(current_song_listen_required));

    let mut current_song_submitted = false;

    loop {
        tokio::select! {
            subsystem = state_changes.next() => {
                match subsystem {
                    Some(Ok(Subsystem::Player | Subsystem::Queue)) => (),
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => {
                        error!(error = ?e, "MPD error");
                        return Err(e.into());
                    }
                    None => break,
                }

                let (new_state, new_song) = get_status_and_song(&mpd_client).await?;

                trace!(old_state = ?state, ?new_state, "possible state change");

                if is_same_song(&current_song, &new_song) {
                    trace!("still the same song");

                    if state != PlayState::Playing && new_state == PlayState::Playing {
                        // Resumed from pause, update the listen start time
                        trace!("resumed from pause or stop");
                        current_song_listen_started = Instant::now();
                        current_song_listen_finished = Box::pin(sleep(current_song_listen_required));
                    } else if state == PlayState::Playing && new_state == PlayState::Paused {
                        // Paused playing, subtract the elapsed time from the required listen
                        // duration
                        let played = current_song_listen_started.elapsed();
                        let remaining = current_song_listen_required.saturating_sub(played);
                        trace!(?played, ?remaining, "paused");
                        current_song_listen_required = remaining;
                    } else if state != PlayState::Stopped && new_state == PlayState::Stopped {
                        // Stopped playing entirely. If the playback starts again with the same
                        // song, count it as a new listen
                        trace!("stopped");
                        current_song_submitted = false;
                        current_song_listen_required = new_song.as_ref().map_or(MAX_REQUIRED_LISTEN_TIME, required_time_for_song);
                    }
                } else {
                    // The song changed
                    debug!(old = ?song_url(&current_song), new = ?song_url(&new_song), "song changed");

                    current_song_submitted = false;
                    current_song_listen_started = Instant::now();
                    current_song_listen_required = new_song.as_ref().map_or(MAX_REQUIRED_LISTEN_TIME, required_time_for_song);
                    current_song_listen_finished = Box::pin(sleep(current_song_listen_required));
                }

                state = new_state;
                current_song = new_song;
            }
            _ = &mut current_song_listen_finished, if state == PlayState::Playing && !current_song_submitted => {
                info!(song = ?song_url(&current_song), "completed required listen duration, sending listen");
                current_song_submitted = true;
            }
        }
    }

    Ok(())
}

async fn get_status_and_song(client: &Client) -> Result<(PlayState, Option<Song>)> {
    client
        .command_list((commands::Status, commands::CurrentSong))
        .await
        .map(|(state, song)| (state.state, song.map(|s| s.song)))
        .map_err(Into::into)
}

/// Calculate the required listen duration for the given song to count as a completed ListenBrainz
/// listen.
fn required_time_for_song(song: &Song) -> Duration {
    let required_time = if let Some(song_duration) = song.duration {
        // A song counts as listened if either half its duration or MAX_REQUIRED_LISTEN_TIME,
        // whichever is lower, was listened to
        cmp::min(song_duration / 2, MAX_REQUIRED_LISTEN_TIME)
    } else {
        warn!("song with unknown duration, assuming 4 minutes listen time");
        MAX_REQUIRED_LISTEN_TIME
    };

    required_time
}

fn song_url(s: &Option<Song>) -> &str {
    s.as_ref().map_or("<none>", |s| &*s.url)
}

fn is_same_song(a: &Option<Song>, b: &Option<Song>) -> bool {
    song_url(a) == song_url(b)
}
