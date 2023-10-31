//! This module contains the type definitions for the ListenBrainz API.

use std::collections::HashMap;

use anyhow::Result;
use bytes::{BufMut, Bytes, BytesMut};
use mpd_client::{responses::Song, tag::Tag};
use serde::Serialize;
use serde_json::value::RawValue;
use tracing::warn;

use crate::{config::Configuration, is_valid_mbid, Feedback};

/// Maximum number of tags the ListenBrainz server will accept.
const MAX_TAGS: usize = 50;

/// Maximum length of a single tag the ListenBrainz server will accept.
const MAX_SINGLE_TAG_LENGTH: usize = 64;

/// Maximum length in bytes of a single listen submission.
const MAX_SERIALIZED_LISTEN_LENGTH: usize = 10240;

/// Maximum number of listens that can be included in an import request. The
/// ListenBrainz server documents a limit of 100, subtract one to ensure
/// remaining space for the surrounding JSON padding
const MAX_LISTENS_PER_IMPORT: usize = 99;

#[derive(Debug, Serialize)]
#[serde(tag = "listen_type", content = "payload")]
enum Submission<'a> {
    #[serde(rename = "import")]
    CompletedListens(&'a [Box<RawValue>]),
    #[serde(rename = "playing_now")]
    PlayingNow([&'a PlayingNow; 1]),
}

#[derive(Debug, Clone)]
pub(super) struct JsonBody(Bytes);

impl JsonBody {
    /// Create a new JsonBody containing the given value
    fn new<V: Serialize>(v: &V) -> JsonBody {
        let mut buf = BytesMut::new();
        serde_json::to_writer((&mut buf).writer(), v).unwrap();
        JsonBody(buf.freeze())
    }

    fn len(&self) -> usize {
        self.0.len()
    }
}

impl From<JsonBody> for reqwest::Body {
    fn from(value: JsonBody) -> Self {
        value.0.into()
    }
}

pub(super) fn prepare_playing_now(config: &Configuration, song: Song) -> Option<JsonBody> {
    let playing_now = PlayingNow {
        track_metadata: metadata_from_song(config, song)?,
    };
    let submission = Submission::PlayingNow([&playing_now]);

    let body = JsonBody::new(&submission);
    if body.len() <= MAX_SERIALIZED_LISTEN_LENGTH {
        Some(body) // once told me ...
    } else {
        warn!(
            length = body.len(),
            "submission would be too large, skipping"
        );
        None
    }
}

pub(super) fn serialize_single_listen(
    config: &Configuration,
    song: Song,
    timestamp: u64,
) -> Option<Box<RawValue>> {
    let listen = Listen {
        track_metadata: metadata_from_song(config, song)?,
        listened_at: timestamp,
    };

    let serialized = serde_json::value::to_raw_value(&listen).unwrap();
    let serialized_length = serialized.get().len();

    if serialized_length <= MAX_SERIALIZED_LISTEN_LENGTH {
        Some(serialized)
    } else {
        warn!(serialized_length, "submission would be too large, skipping");
        None
    }
}

pub(super) fn prepare_completed_listens(listens: &[Box<RawValue>]) -> JsonBody {
    assert!(listens.len() <= MAX_LISTENS_PER_IMPORT);

    let submission = Submission::CompletedListens(listens);
    JsonBody::new(&submission)
}

pub(super) fn prepare_recording_mbid_lookup(song: Song) -> Result<LookupRecordingMbid> {
    let mut tags = song.tags;
    let song = &song.url;

    let Some(artist_name) = single_value(&mut tags, Tag::Artist, song) else {
        anyhow::bail!("Cannot look up track without artist tag");
    };

    let Some(recording_name) = single_value(&mut tags, Tag::Title, song) else {
        anyhow::bail!("Cannot look up track without title tag");
    };

    Ok(LookupRecordingMbid {
        recording_name,
        artist_name,
    })
}

pub(super) fn prepare_feedback_submission(
    recording_mbid: String,
    feedback: Feedback,
) -> SubmitFeedback {
    SubmitFeedback {
        recording_mbid,
        feedback,
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
        tags: if config.submit_genres_as_folksonomy {
            folksonomy_tags(&mut tags, config.genre_separator)
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
pub struct LookupRecordingMbid {
    recording_name: String,
    artist_name: String,
}

#[derive(Debug, Serialize)]
pub struct SubmitFeedback {
    recording_mbid: String,
    #[serde(rename = "score")]
    feedback: Feedback,
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
