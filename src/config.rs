use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::debug;

/// The default configuration file.
pub(crate) const DEFAULT: &str = include_str!("../config.toml.sample");

pub(crate) fn default_path() -> PathBuf {
    let mut p = dirs::config_dir().expect("no config directory on this platform");
    p.push(env!("CARGO_PKG_NAME"));
    p.push("config.toml");
    p
}

pub(crate) fn load(path: &Path) -> Result<Configuration> {
    debug!(?path, "loading configuration file");

    let config = fs::read(path)
        .with_context(|| format!("Failed to read configuration file at {}", path.display()))?;

    let mut config: Configuration = toml::from_slice(&config)
        .with_context(|| format!("Failed to parse configuration file at {}", path.display()))?;

    if let Some(pw) = &config.mpd.password {
        if pw.is_empty() {
            config.mpd.password = None;
        }
    }

    Ok(config)
}

#[derive(Debug, Deserialize)]
pub(crate) struct Configuration {
    #[serde(rename = "listenbrainz_token")]
    pub(crate) token: String,
    #[serde(default = "default_api_url")]
    pub(crate) api_url: String,
    #[serde(default)]
    pub(crate) mpd: Mpd,
    #[serde(default)]
    pub(crate) submission: Submission,
}

fn default_api_url() -> String {
    String::from("https://api.listenbrainz.org")
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub(crate) struct Mpd {
    pub(crate) address: String,
    pub(crate) password: Option<String>,
}

impl Default for Mpd {
    fn default() -> Self {
        Mpd {
            address: String::from("127.0.0.1:6600"),
            password: None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub(crate) struct Submission {
    pub(crate) genres_as_folksonomy: bool,
    pub(crate) genre_separator: Option<char>,
}

impl Default for Submission {
    fn default() -> Self {
        Submission {
            genres_as_folksonomy: true,
            genre_separator: None,
        }
    }
}
