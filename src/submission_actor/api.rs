//! This module contains the type definitions for the ListenBrainz API.

use std::collections::HashMap;

use mpd_client::{commands::responses::Song, Tag};
use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Debug, Deserialize)]
pub(super) struct ValidateToken {
    pub(super) valid: bool,
    #[serde(default)]
    pub(super) user_name: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "listen_type", content = "payload")]
pub(super) enum Submission {
    #[serde(rename = "single")]
    Listen([Listen; 1]),
    #[serde(rename = "playing_now")]
    PlayingNow([PlayingNow; 1]),
}

impl Submission {
    pub(super) fn listen(song: Song, timestamp: u64) -> Option<Submission> {
        Some(Submission::Listen([Listen {
            listened_at: timestamp,
            track_metadata: metadata_from_song(song)?,
        }]))
    }

    pub(super) fn playing_now(song: Song) -> Option<Submission> {
        Some(Submission::PlayingNow([PlayingNow {
            track_metadata: metadata_from_song(song)?,
        }]))
    }
}

fn metadata_from_song(song: Song) -> Option<TrackMetadata> {
    let mut tags = song.tags;
    let song = song.url.as_str();

    let artist_name = if let Some(a) = single_value(&mut tags, Tag::Artist, song) {
        a
    } else {
        warn!(song, "cannot submit track without artist tag");
        return None;
    };

    let track_name = if let Some(a) = single_value(&mut tags, Tag::Title, song) {
        a
    } else {
        warn!(song, "cannot submit track without title tag");
        return None;
    };

    let release_name = single_value(&mut tags, Tag::Album, song);

    let additional_info = AdditionalInfo {
        artist_mbids: tags.remove(&Tag::MusicBrainzArtistId).unwrap_or_default(),
        release_mbid: single_value(&mut tags, Tag::MusicBrainzReleaseId, song),
        recording_mbid: single_value(&mut tags, Tag::MusicBrainzRecordingId, song),
        track_mbid: single_value(&mut tags, Tag::MusicBrainzTrackId, song),
        work_mbids: tags.remove(&Tag::MusicBrainzWorkId).unwrap_or_default(),
        tracknumber: single_value(&mut tags, Tag::Track, song),
        tags: tags.remove(&Tag::Genre).unwrap_or_default(),
        media_player: "MPD",
        submission_client: env!("CARGO_PKG_NAME"),
        submission_client_version: env!("CARGO_PKG_VERSION"),
    };

    Some(TrackMetadata {
        artist_name,
        track_name,
        release_name,
        additional_info,
    })
}

fn single_value(tags: &mut HashMap<Tag, Vec<String>>, tag: Tag, song: &str) -> Option<String> {
    if let Some(mut v) = tags.remove(&tag) {
        if v.is_empty() {
            return None;
        } else if v.len() > 1 {
            warn!(
                song,
                ?tag,
                "multiple values for tag, only sending the first"
            );
        }

        Some(v.remove(0))
    } else {
        None
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct Listen {
    listened_at: u64,
    track_metadata: TrackMetadata,
}

#[derive(Debug, Serialize)]
pub(crate) struct PlayingNow {
    track_metadata: TrackMetadata,
}

#[derive(Debug, Serialize)]
struct TrackMetadata {
    artist_name: String,
    track_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    release_name: Option<String>,
    additional_info: AdditionalInfo,
}

#[derive(Debug, Serialize)]
struct AdditionalInfo {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    artist_mbids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    release_mbid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recording_mbid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    track_mbid: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    work_mbids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tracknumber: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    media_player: &'static str,
    submission_client: &'static str,
    submission_client_version: &'static str,
}
