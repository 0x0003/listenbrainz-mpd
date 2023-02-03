//! This module contains the type definitions for the ListenBrainz API.

use std::collections::HashMap;

use mpd_client::{responses::Song, tag::Tag};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::config::Configuration;

/// Maximum number of tags the ListenBrainz server will accept.
const MAX_TAGS: usize = 50;

/// Maximum length of a single tag the ListenBrainz server will accept.
const MAX_SINGLE_TAG_LENGTH: usize = 64;

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
    pub(super) fn listen(config: &Configuration, song: Song, timestamp: u64) -> Option<Submission> {
        Some(Submission::Listen([Listen {
            listened_at: timestamp,
            track_metadata: metadata_from_song(config, song)?,
        }]))
    }

    pub(super) fn playing_now(config: &Configuration, song: Song) -> Option<Submission> {
        Some(Submission::PlayingNow([PlayingNow {
            track_metadata: metadata_from_song(config, song)?,
        }]))
    }
}

fn metadata_from_song(config: &Configuration, song: Song) -> Option<TrackMetadata> {
    let mut tags = song.tags;
    let duration = song.duration;
    let song = song.url.as_str();

    let Some(artist_name) = single_value(&mut tags, Tag::Artist, song) else {
        warn!(song, "cannot submit track without artist tag");
        return None;
    };

    let Some(track_name) = single_value(&mut tags, Tag::Title, song) else {
        warn!(song, "cannot submit track without title tag");
        return None;
    };

    let release_name = single_value(&mut tags, Tag::Album, song);

    let mut additional_info = AdditionalInfo {
        artist_mbids: tags.remove(&Tag::MusicBrainzArtistId).unwrap_or_default(),
        release_mbid: single_value(&mut tags, Tag::MusicBrainzReleaseId, song),
        recording_mbid: single_value(&mut tags, Tag::MusicBrainzRecordingId, song),
        track_mbid: single_value(&mut tags, Tag::MusicBrainzTrackId, song),
        work_mbids: tags.remove(&Tag::MusicBrainzWorkId).unwrap_or_default(),
        tracknumber: single_value(&mut tags, Tag::Track, song),
        duration_ms: duration.map(|d| d.as_millis()),
        tags: if config.submission.genres_as_folksonomy {
            folksonomy_tags(&mut tags, config.submission.genre_separator)
        } else {
            Vec::new()
        },
        media_player: "MPD",
        submission_client: env!("CARGO_PKG_NAME"),
        submission_client_version: env!("CARGO_PKG_VERSION"),
    };

    additional_info.validate_mbids();

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

fn folksonomy_tags(
    tags: &mut HashMap<Tag, Vec<String>>,
    value_separator: Option<char>,
) -> Vec<String> {
    let genres = tags.remove(&Tag::Genre).unwrap_or_default();

    let mut out = if let Some(value_separator) = value_separator {
        let mut out = Vec::with_capacity(genres.len());

        for v in genres {
            out.extend(v.split(value_separator).map(str::trim).map(String::from));
        }

        if out.len() > MAX_TAGS {
            warn!(tags = out.len(), "too many tags, ignoring excess values");
            out.truncate(MAX_TAGS);
        }

        out
    } else {
        genres
    };

    out.retain(|tag| {
        if tag.len() < MAX_SINGLE_TAG_LENGTH {
            true
        } else {
            warn!(?tag, "oversized folksonomy tag, ignoring");
            false
        }
    });

    if out.len() > MAX_TAGS {
        warn!(
            tags = out.len(),
            "too many folksonomy tags, ignoring excess values"
        );
        out.truncate(MAX_TAGS);
    }

    out
}

#[derive(Debug, Serialize)]
pub struct Listen {
    listened_at: u64,
    track_metadata: TrackMetadata,
}

#[derive(Debug, Serialize)]
pub struct PlayingNow {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u128>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    media_player: &'static str,
    submission_client: &'static str,
    submission_client_version: &'static str,
}

impl AdditionalInfo {
    fn validate_mbids(&mut self) {
        validate_multiple_mbid(&mut self.artist_mbids);
        validate_single_mbid(&mut self.release_mbid);
        validate_single_mbid(&mut self.recording_mbid);
        validate_single_mbid(&mut self.track_mbid);
        validate_multiple_mbid(&mut self.work_mbids);
    }
}

fn validate_single_mbid(val: &mut Option<String>) {
    if let Some(mbid) = val {
        if !is_valid_mbid(mbid) {
            warn!(?mbid, "invalid MBID, ignoring");
            *val = None;
        }
    }
}

fn validate_multiple_mbid(vals: &mut Vec<String>) {
    vals.retain(|mbid| {
        let is_valid = is_valid_mbid(mbid);

        if !is_valid {
            warn!(?mbid, "invalid MBID, ignoring");
        }

        is_valid
    });
}

/// Validate that a given MBID string conforms to the expected format (dashed lowercase).
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
