mod cache_actor;
mod cli;
mod config;
mod submission_actor;

use std::{
    cmp,
    path::Path,
    pin::Pin,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use clap::Parser;
use config::Configuration;
use mpd_client::{
    client::{Client, ConnectionEvent, ConnectionEvents, Subsystem},
    commands,
    responses::{PlayState, Song, SongInQueue},
    tag::Tag,
};
use serde::{Serialize, Serializer};
use tokio::{
    net::{TcpStream, UnixStream},
    signal::ctrl_c,
    time::{sleep, Sleep},
};
use tracing::{debug, error, info, info_span, trace, warn, Instrument};
use tracing_subscriber::EnvFilter;

use crate::{
    cache_actor::CacheActor,
    cli::{CliArgs, Feedback},
    submission_actor::SubmissionActor,
};

/// The maximum time you have to listen to a song before it will count as a
/// listen. Set to 4 minutes as per the recommendations in the ListenBrainz
/// documentation.
const MAX_REQUIRED_LISTEN_TIME: Duration = Duration::from_secs(4 * 60);

/// Name of the client-to-client channel used to send ListenBrainz feedback
/// commands.
const FEEDBACK_CHANNEL_NAME: &str = "listenbrainz_feedback";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_env("LISTENBRAINZ_MPD_LOG"))
        .init();

    let args = CliArgs::parse();

    if args.create_default_config {
        return config::create_default_config();
    }

    let config = config::load(args.config).context("Failed to load configuration")?;

    let cache_actor = CacheActor::start(&config)?;
    let (mpd_client, state_changes) = connect(&config).await?;
    let (http_actor, http_actor_handle) = SubmissionActor::start(config, cache_actor)?;

    if let Some(feedback) = args.send_feedback {
        return send_feedback(mpd_client, feedback).await;
    }

    let res = run(mpd_client, state_changes, http_actor).await;

    #[cfg(feature = "systemd")]
    {
        let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Stopping]);
    }

    // Wait for actors to exit
    http_actor_handle.await.expect("HTTP actor panicked");

    res
}

async fn send_feedback(mpd_client: Client, feedback: Feedback) -> Result<()> {
    mpd_client
        .command(commands::SendChannelMessage::new(
            FEEDBACK_CHANNEL_NAME,
            feedback.as_command(),
        ))
        .await
        .context("Failed to send feedback message (Is a daemon instance running?)")?;

    Ok(())
}

async fn connect(config: &Configuration) -> Result<(Client, ConnectionEvents)> {
    let password = config.mpd_password.as_deref();

    if config.mpd_host.starts_with('/') {
        // If the host value starts with a slash, assume it's a path to a Unix socket
        connect_unix(Path::new(&config.mpd_host), password)
            .await
            .with_context(|| format!("Failed to connect via Unix socket at {}", config.mpd_host))
    } else {
        // Otherwise assume it's a hostname or bare IP address
        connect_tcp(&config.mpd_host, config.mpd_port, password)
            .await
            .with_context(|| {
                format!(
                    "Failed to connect via TCP to {} port {}",
                    config.mpd_host, config.mpd_port
                )
            })
    }
}

async fn connect_tcp(
    host: &str,
    port: u16,
    password: Option<&str>,
) -> Result<(Client, ConnectionEvents)> {
    debug!(?host, port, "connecting via TCP");
    let socket = TcpStream::connect((host, port)).await?;
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
    /// The point in time at which the current listen segment was started. This
    /// is used to calculate the real elapsed time when processing
    /// pauses/unpauses.
    listen_started: Instant,
    /// The system timestamp when the listen was started. This is used during
    /// submission to the ListenBrainz API.
    listen_timestamp: SystemTime,
    /// The required remaining time the current song needs to play before it
    /// will count as a listen.
    listen_required: Duration,
    /// The future that completes when the required duration is reached.
    listen_finished: Pin<Box<Sleep>>,
    /// `true` if a listen record for the current song has already been
    /// submitted.
    listen_submitted: bool,
    /// Counter for completed listens
    completed_listens: u64,
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

    // Subscribe to the client-to-client channel used for feedback
    mpd_client
        .command(commands::SubscribeToChannel(FEEDBACK_CHANNEL_NAME))
        .await?;

    let mut state = State {
        play_state,
        song,
        listen_started: Instant::now(),
        listen_timestamp: SystemTime::now(),
        listen_required,
        listen_finished: Box::pin(sleep(listen_required)),
        listen_submitted: false,
        completed_listens: 0,
    };

    #[cfg(feature = "systemd")]
    let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Ready]);

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
        #[cfg(feature = "systemd")]
        let _ = sd_notify::notify(
            false,
            &[sd_notify::NotifyState::Status(&format!(
                "Watching for listens; {} completed",
                state.completed_listens
            ))],
        );

        tokio::select! {
            event = connection_events.next() => {
                match event {
                    Some(ConnectionEvent::SubsystemChange(subsystem)) => {
                        handle_subsystem_event(
                            subsystem,
                            &mut state,
                            &mpd_client,
                            &http_actor,
                        ).await?;
                    }
                    Some(ConnectionEvent::ConnectionClosed(e)) => {
                        error!(error = ?e, "MPD error");
                        return Err(e.into());
                    }
                    None => {
                        debug!("MPD server closed connection");
                        return Ok(());
                    }
                }
            }
            _ = &mut state.listen_finished, if state.should_poll() => {
                handle_listen_complete(&mut state, &http_actor);
            }
            _ = ctrl_c() => {
                debug!("received interrupt");
                return Ok(());
            }
        }
    }
}

async fn handle_subsystem_event(
    subsystem: Subsystem,
    state: &mut State,
    mpd_client: &Client,
    http_actor: &SubmissionActor,
) -> Result<()> {
    trace!(?subsystem, "Subsystem change");
    match subsystem {
        // Something about the player changed (e.g. play state, current song)
        Subsystem::Player | Subsystem::Queue => {
            handle_state_change(state, mpd_client, http_actor.clone()).await
        }
        // Received a message on one of our subscribed channels (for feedback)
        Subsystem::Message => handle_message_event(state, mpd_client, http_actor.clone()).await,
        // Nothing relevant for us
        _ => Ok(()),
    }
}

async fn handle_state_change(
    state: &mut State,
    mpd_client: &Client,
    http_actor: SubmissionActor,
) -> Result<()> {
    let (new_play_state, new_song) = get_status_and_song(mpd_client).await?;
    let is_single_on = get_single_status(mpd_client).await?;

    let same_song = is_same_song(state.song.as_ref(), new_song.as_ref());

    if same_song && state.play_state == new_play_state {
        if is_single_on == commands::SingleMode::Enabled {
            trace!("single onsus");
        } else {
            trace!("single offosus");
        }
        // Nothing relevant changed. This happens e.g. when the status of the repeat or
        // shuffle options is changed
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
            if new_play_state == PlayState::Playing {
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
    state.completed_listens += 1;

    let song = state.song.clone().expect("no song to submit");

    let timestamp = state
        .listen_timestamp
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    http_actor.listen(song.song, timestamp);
}

async fn handle_message_event(
    state: &State,
    mpd_client: &Client,
    http_actor: SubmissionActor,
) -> Result<()> {
    // Read our messages
    let messages = mpd_client
        .command(commands::ReadChannelMessages)
        .await
        .context("Failed to read messages")?;

    let Some((_, message)) = messages
        .into_iter()
        .find(|(channel, _)| channel == FEEDBACK_CHANNEL_NAME)
    else {
        debug!("no feedback message");
        return Ok(());
    };
    debug!(?message, "feedback command received");

    let Some(feedback) = Feedback::from_command(&message) else {
        warn!(?message, "invalid feedback command, ignoring");
        return Ok(());
    };

    let Some(song) = state.song.clone().map(|s| s.song) else {
        debug!("no current song to submit feedback for");
        return Ok(());
    };

    let span = info_span!("submit_feedback", ?feedback, song = ?song.url);
    tokio::spawn(
        async move {
            if let Err(error) = submit_feedback(song, http_actor, feedback).await {
                error!(?error, "Failed to submit feedback");
            }
        }
        .instrument(span),
    );

    Ok(())
}

async fn submit_feedback(
    mut song: Song,
    http_actor: SubmissionActor,
    feedback: Feedback,
) -> Result<()> {
    debug!("submitting feedback");

    let mbid = song
        .tags
        .remove(&Tag::MusicBrainzRecordingId)
        .and_then(|mut v| {
            trace!("found existing recording MBID tag");
            if v.len() > 1 {
                warn!(
                    values = v.len(),
                    "more than one recording MBID tag, ignoring all but the first"
                );
            }

            let mbid = v.remove(0);

            if is_valid_mbid(&mbid) {
                Some(mbid)
            } else {
                warn!("invalid recording MBID, ignoring");
                None
            }
        });

    let mbid = if let Some(mbid) = mbid {
        mbid
    } else {
        debug!("requesting MBID mapping from ListenBrainz API");
        http_actor
            .lookup_recording_mbid(song)
            .await
            .context("Failed to look up MBID mapping for recording")?
    };

    trace!(?mbid);
    http_actor
        .submit_feedback(mbid, feedback)
        .await
        .context("Failed to submit feedback")
}

impl Serialize for Feedback {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_i8(match self {
            Feedback::Love => 1,
            Feedback::Hate => -1,
            Feedback::Clear => 0,
        })
    }
}

fn is_same_song(a: Option<&SongInQueue>, b: Option<&SongInQueue>) -> bool {
    let Some((a, b)) = a.zip(b) else { return false };
    a.id == b.id && a.position == b.position && a.song.url == b.song.url
}

async fn get_status_and_song(client: &Client) -> Result<(PlayState, Option<SongInQueue>)> {
    client
        .command_list((commands::Status, commands::CurrentSong))
        .await
        .map(|(state, song)| (state.state, song))
        .map_err(Into::into)
}

async fn get_single_status(client: &Client) -> Result<commands::SingleMode> {
    client
        .command(commands::Status)
        .await
        .map(|state| (state.single))
        .map_err(Into::into)
}

/// Calculate the required listen duration for the given song to count as a
/// completed ListenBrainz listen.
fn required_time_for_song(song: Option<&SongInQueue>) -> Duration {
    if let Some(s) = song {
        if let Some(song_duration) = s.song.duration {
            // A song counts as listened if either half its duration or
            // MAX_REQUIRED_LISTEN_TIME, whichever is lower, was listened to
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

/// Validate that a given MBID string conforms to the expected format (dashed
/// lowercase).
fn is_valid_mbid(mbid: &str) -> bool {
    if mbid.len() != 36 {
        return false;
    }

    for range in [0..8, 9..13, 14..18, 19..23, 24..36] {
        if mbid[range].chars().any(|c| !c.is_ascii_alphanumeric()) {
            return false;
        }
    }

    for dash_position in [8, 13, 18, 23] {
        if &mbid[dash_position..=dash_position] != "-" {
            return false;
        }
    }

    true
}
