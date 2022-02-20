mod config;

use std::{
    cmp,
    net::SocketAddr,
    path::Path,
    pin::Pin,
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
    time::{sleep, Sleep},
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

#[derive(Debug)]
struct State {
    /// Current play state of the server.
    play_state: PlayState,
    /// The current playing song, if any.
    song: Option<Song>,
    /// The point in time at which the current listen segment was started. This is used to
    /// calculate the real elapsed time when processing pauses/unpauses.
    listen_started: Instant,
    /// The required remaining time the current song needs to play before it will count as a
    /// listen.
    listen_required: Duration,
    /// The future that completes when the required duration is reached.
    listen_finished: Pin<Box<Sleep>>,
    /// `true` if a listen record for the current song has already been submitted.
    listen_submitted: bool,
}

impl State {
    fn should_poll(&self) -> bool {
        self.play_state == PlayState::Playing && !self.listen_submitted
    }
}

async fn run(
    mpd_client: Client,
    mut state_changes: StateChanges,
    http_client: reqwest::Client,
) -> Result<()> {
    // Setup initial state
    let (play_state, song) = get_status_and_song(&mpd_client).await?;

    let listen_required = required_time_for_song(song.as_ref());

    let mut state = State {
        play_state,
        song,
        listen_started: Instant::now(),
        listen_required,
        listen_finished: Box::pin(sleep(listen_required)),
        listen_submitted: false,
    };

    if state.song.is_some() {
        debug!(
            song = song_url(state.song.as_ref()),
            required_playtime = ?listen_required,
            "starting with initial song"
        );
    }

    debug!("entering main loop");

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
                    None => {
                        info!("MPD server closed connection; exiting");
                        break;
                    }
                }

                handle_state_change(&mut state, &mpd_client).await?;
            }
            _ = &mut state.listen_finished, if state.should_poll() => {
                handle_listen_complete(&mut state, http_client.clone()).await;
            }
        }
    }

    Ok(())
}

async fn handle_state_change(state: &mut State, mpd_client: &Client) -> Result<()> {
    let (new_play_state, new_song) = get_status_and_song(mpd_client).await?;

    let same_song = state.song.as_ref().map(|s| &s.url) == new_song.as_ref().map(|s| &s.url);

    if same_song && state.play_state == new_play_state {
        // Nothing relevant changed. This happens e.g. when the status of the repeat or shuffle
        // options is changed
        trace!("nothing changed");
    } else if same_song {
        if state.play_state != PlayState::Playing && new_play_state == PlayState::Playing {
            // Resumed from pause, update the listen start time
            trace!("resumed from pause or stop");
            state.listen_started = Instant::now();
            state.listen_finished = Box::pin(sleep(state.listen_required));
        } else if state.play_state == PlayState::Playing && new_play_state == PlayState::Paused {
            // Paused playing, subtract the elapsed time from the required listen
            // duration
            let played = state.listen_started.elapsed();
            let remaining = state.listen_required.saturating_sub(played);
            trace!(?played, ?remaining, "paused");
            state.listen_required = remaining;
        } else if state.play_state != PlayState::Stopped && new_play_state == PlayState::Stopped {
            // Stopped playing entirely. If the playback starts again with the same
            // song, count it as a new listen
            trace!("stopped");
            state.listen_submitted = false;
            state.listen_required = required_time_for_song(new_song.as_ref());
        }
    } else {
        // The song changed
        let required_playtime = required_time_for_song(new_song.as_ref());
        debug!(
            song = song_url(new_song.as_ref()),
            ?required_playtime,
            "song changed"
        );

        state.listen_started = Instant::now();
        state.listen_required = required_playtime;
        state.listen_finished = Box::pin(sleep(required_playtime));
        state.listen_submitted = false;
    }

    state.play_state = new_play_state;
    state.song = new_song;

    Ok(())
}

async fn handle_listen_complete(state: &mut State, _http_client: reqwest::Client) {
    info!(
        song = song_url(state.song.as_ref()),
        "submitting listen entry"
    );
    state.listen_submitted = true;
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
fn required_time_for_song(song: Option<&Song>) -> Duration {
    let required_time = if let Some(song) = song {
        if let Some(song_duration) = song.duration {
            // A song counts as listened if either half its duration or MAX_REQUIRED_LISTEN_TIME,
            // whichever is lower, was listened to
            cmp::min(song_duration / 2, MAX_REQUIRED_LISTEN_TIME)
        } else {
            warn!("song with unknown duration, assuming 4 minutes listen time");
            MAX_REQUIRED_LISTEN_TIME
        }
    } else {
        MAX_REQUIRED_LISTEN_TIME
    };

    required_time
}

fn song_url(s: Option<&Song>) -> &str {
    s.map_or("<none>", |s| &*s.url)
}
