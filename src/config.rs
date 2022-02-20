use std::{fs, path::Path};

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use tracing::debug;

pub(crate) fn load(path: &Path) -> Result<Configuration> {
    debug!(?path, "loading configuration file");

    let config = fs::read(path)
        .with_context(|| format!("reading configuration file at {}", path.display()))?;

    let mut config: Configuration = toml::from_slice(&config)
        .with_context(|| format!("parsing configuration file at {}", path.display()))?;

    if config.mpd.address.is_empty() {
        bail!("MPD address cannot be empty");
    }

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
    #[serde(default)]
    pub(crate) mpd: Mpd,
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
