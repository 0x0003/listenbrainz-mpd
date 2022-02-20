use std::{
    fs,
    net::{IpAddr, Ipv4Addr},
    num::NonZeroU16,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use tracing::debug;

pub(crate) fn load(path: &Path) -> Result<Configuration> {
    debug!(?path, "loading configuration file");

    let config = fs::read("config.toml")
        .with_context(|| format!("reading configuration file at {}", path.display()))?;

    let config: Configuration = toml::from_slice(&config)
        .with_context(|| format!("parsing configuration file at {}", path.display()))?;

    if let Some(pw) = &config.mpd.password {
        if pw.is_empty() {
            bail!("password cannot be empty");
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
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Mpd {
    pub(crate) password: Option<String>,
    #[serde(flatten)]
    pub(crate) connection: MpdConnection,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum MpdConnection {
    UnixSocket {
        unix: PathBuf,
    },
    Tcp {
        ip: IpAddr,
        #[serde(default = "default_port")]
        port: NonZeroU16,
    },
}

fn default_port() -> NonZeroU16 {
    NonZeroU16::new(6600).unwrap()
}

impl Default for MpdConnection {
    fn default() -> Self {
        MpdConnection::Tcp {
            ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: default_port(),
        }
    }
}
