mod config;
mod submission_actor;

use std::{
    cmp,
    path::{Path, PathBuf},
    pin::Pin,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use clap::{ArgAction, Parser};
use mpd_client::{
    client::{Client, ConnectionEvent, ConnectionEvents, Subsystem},
    commands,
    responses::{PlayState, Song, SongInQueue},
};
use tokio::{
    net::{TcpStream, UnixStream},
    time::{sleep, Sleep},
};
use tracing::{debug, error, info, trace, warn};
use tracing_subscriber::EnvFilter;

use crate::submission_actor::SubmissionActor;

/// The maximum time you have to listen to a song before it will count as a listen. Set to 4
/// minutes as per the recommendations in the ListenBrainz documentation.
const MAX_REQUIRED_LISTEN_TIME: Duration = Duration::from_secs(4 * 60);

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_env("LISTENBRAINZ_MPD_LOG"))
        .init();

    let args = CliArgs::parse();

    if args.create_default_config {
        return config::create_default_config();
    }

    let config = config::load(args)?;

    let (mpd_client, state_changes) = connect(&config.mpd).await?;
    let http_actor = SubmissionActor::start(config).await?;

    run(mpd_client, state_changes, http_actor).await
}

#[derive(Parser)]
#[clap(version, about)]
pub struct CliArgs {
    /// Path to the configuration file.
    #[clap(short, long)]
    config: Option<PathBuf>,
    /// Create a configuration file in the default location and exit
    #[clap(long, action = ArgAction::SetTrue, exclusive = true)]
    create_default_config: bool,
}

async fn connect(mpd_config: &config::Mpd) -> Result<(Client, ConnectionEvents)> {
    let password = mpd_config.password.as_deref();

    if mpd_config.address.starts_with('/') {
        // If the address value starts with a slash, assume it's a path to a Unix socket
        connect_unix(Path::new(&mpd_config.address), password)
            .await
            .with_context(|| {
                format!(
                    "Failed to connect via Unix socket at {}",
                    mpd_config.address
                )
            })
    } else {
        // Otherwise assume it's an IP address/hostname
        connect_tcp(&mpd_config.address, password)
            .await
            .with_context(|| format!("Failed to connect via TCP to {}", mpd_config.address))
    }
}

async fn connect_tcp(address: &str, password: Option<&str>) -> Result<(Client, ConnectionEvents)> {
    debug!(?address, "connecting via TCP");

    let (address, port) = address.rsplit_once(':').unwrap_or((address, "6600"));

    let port = port.parse().context("Failed to parse port")?;

    let socket = TcpStream::connect((address, port)).await?;
    Client::connect_with_password_opt(socket, password)
        .await
        .map_err(Into::into)
}

async fn connect_unix(path: &Path, password: Option<&str>) -> Result<(Client, ConnectionEvents)> {
    debug!(?path, "connecting via Unix socket");
    let socket = UnixStream::connect(path).await?;
    Client::connect_with_password_opt(socket, password)
        .await
        .map_err(Into::into)
}

#[derive(Debug)]
struct State {
    /// Current play state of the server.
    play_state: PlayState,
    /// The current playing song, if any.
    song: Option<SongInQueue>,
    /// The point in time at which the current listen segment was started. This is used to
    /// calculate the real elapsed time when processing pauses/unpauses.
    listen_started: Instant,
    /// The system timestamp when the listen was started. This is used during submission to the
    /// ListenBrainz API.
    listen_timestamp: SystemTime,
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
    mut connection_events: ConnectionEvents,
    http_actor: SubmissionActor,
) -> Result<()> {
    // Setup initial state
    let (play_state, song) = get_status_and_song(&mpd_client).await?;

    let listen_required = required_time_for_song(song.as_ref());

    let mut state = State {
        play_state,
        song,
        listen_started: Instant::now(),
        listen_timestamp: SystemTime::now(),
        listen_required,
        listen_finished: Box::pin(sleep(listen_required)),
        listen_submitted: false,
    };

    // Send initial now_playing if we start while a song is playing
    if let Some(song) = &state.song {
        if state.play_state == PlayState::Playing {
            debug!(
                song = %song.song.url,
                required_playtime = ?listen_required,
                "starting with initial song"
            );
            http_actor.now_playing(song.song.clone());
        }
    }

    debug!("entering main loop");

    loop {
        tokio::select! {
            event = connection_events.next() => {
                match event {
                    Some(ConnectionEvent::SubsystemChange(Subsystem::Player | Subsystem::Queue)) => (),
                    Some(ConnectionEvent::SubsystemChange(_)) => continue,
                    Some(ConnectionEvent::ConnectionClosed(e)) => {
                        error!(error = ?e, "MPD error");
                        return Err(e.into());
                    }
                    None => {
                        debug!("MPD server closed connection");
                        return Ok(());
                    }
                }

                handle_state_change(&mut state, &mpd_client, http_actor.clone()).await?;
            }
            _ = &mut state.listen_finished, if state.should_poll() => {
                handle_listen_complete(&mut state, &http_actor);
            }
        }
    }
}

async fn handle_state_change(
    state: &mut State,
    mpd_client: &Client,
    http_actor: SubmissionActor,
) -> Result<()> {
    let (new_play_state, new_song) = get_status_and_song(mpd_client).await?;

    let same_song = is_same_song(state.song.as_ref(), new_song.as_ref());

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
            song = song_url(new_song.as_ref().map(|s| &s.song)),
            ?required_playtime,
            "song changed"
        );

        state.listen_started = Instant::now();
        state.listen_timestamp = SystemTime::now();
        state.listen_required = required_playtime;
        state.listen_finished = Box::pin(sleep(required_playtime));
        state.listen_submitted = false;

        if let Some(song) = &new_song {
            if state.play_state == PlayState::Playing {
                http_actor.now_playing(song.song.clone());
            }
        }
    }

    state.play_state = new_play_state;
    state.song = new_song;

    Ok(())
}

fn handle_listen_complete(state: &mut State, http_actor: &SubmissionActor) {
    info!(
        song = song_url(state.song.as_ref().map(|s| &s.song)),
        "submitting listen entry"
    );
    state.listen_submitted = true;

    let song = state.song.clone().expect("no song to submit");

    let timestamp = state
        .listen_timestamp
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    http_actor.listen(song.song, timestamp);
}

fn is_same_song(a: Option<&SongInQueue>, b: Option<&SongInQueue>) -> bool {
    let Some((a, b)) = a.zip(b) else { return false; };
    a.id == b.id && a.position == b.position && a.song.url == b.song.url
}

async fn get_status_and_song(client: &Client) -> Result<(PlayState, Option<SongInQueue>)> {
    client
        .command_list((commands::Status, commands::CurrentSong))
        .await
        .map(|(state, song)| (state.state, song))
        .map_err(Into::into)
}

/// Calculate the required listen duration for the given song to count as a completed ListenBrainz
/// listen.
fn required_time_for_song(song: Option<&SongInQueue>) -> Duration {
    if let Some(s) = song {
        if let Some(song_duration) = s.song.duration {
            // A song counts as listened if either half its duration or MAX_REQUIRED_LISTEN_TIME,
            // whichever is lower, was listened to
            cmp::min(song_duration / 2, MAX_REQUIRED_LISTEN_TIME)
        } else {
            warn!("song with unknown duration, assuming 4 minutes listen time");
            MAX_REQUIRED_LISTEN_TIME
        }
    } else {
        MAX_REQUIRED_LISTEN_TIME
    }
}

fn song_url(s: Option<&Song>) -> &str {
    s.map_or("<none>", |s| &*s.url)
}
