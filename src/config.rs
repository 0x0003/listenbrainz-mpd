use std::{
    fs::{self, File},
    io::{ErrorKind, Write},
    path::PathBuf,
};

use anyhow::{anyhow, bail, Context, Error, Result};
use serde::Deserialize;
use tracing::debug;

use crate::CliArgs;

/// The default configuration file.
pub const DEFAULT: &[u8] = include_str!("../config.toml.sample").as_bytes();

fn default_path() -> PathBuf {
    let mut p = dirs::config_dir().expect("no config directory on this platform");
    p.push(env!("CARGO_PKG_NAME"));
    p.push("config.toml");
    p
}

pub fn load(args: CliArgs) -> Result<Configuration> {
    let path = &args.config.unwrap_or_else(default_path);

    debug!(?path, "loading configuration file");

    let config = fs::read_to_string(path)
        .with_context(|| format!("Failed to read configuration file at {}", path.display()))?;

    let mut config: Configuration = toml::from_str(&config)
        .with_context(|| format!("Failed to parse configuration file at {}", path.display()))?;

    if let Token::File { token_file } = &config.submission.token {
        let token = fs::read_to_string(token_file)
            .with_context(|| format!("Failed to read token file {}", token_file.display()))?;
        config.submission.token = Token::Inline { token };
    }

    if config.mpd.password.is_none() {
        if let Some(pw_file) = &config.mpd.password_file {
            let password = fs::read_to_string(pw_file)
                .with_context(|| format!("Failed to read password file {}", pw_file.display()))?;
            config.mpd.password = Some(password);
        }
    }

    validate(&mut config).context("Invalid configuration")?;

    Ok(config)
}

fn validate(config: &mut Configuration) -> Result<()> {
    if config.submission.token.value().is_empty() {
        bail!("User token cannot be empty");
    }

    if config.submission.api_url.is_empty() {
        bail!("API URL cannot be empty");
    }

    if config.mpd.address.is_empty() {
        bail!("MPD address cannot be empty");
    }

    if let Some(pw) = &config.mpd.password {
        if pw.is_empty() {
            config.mpd.password = None;
        }
    }

    Ok(())
}

pub fn create_default_config() -> Result<()> {
    let path = default_path();

    // Create directories if necessary
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)
            .with_context(|| format!("Failed to create directories: {}", p.display()))?;
    }

    // Create the actual config file and write the contents into it, but only if it does not
    // already exist
    match File::options().write(true).create_new(true).open(&path) {
        Ok(mut f) => {
            f.write_all(DEFAULT).with_context(|| {
                format!(
                    "Failed to write to the newly created configuration file at {}",
                    path.display()
                )
            })?;
            f.flush()?;

            println!(
                "Created new default configuration file at {}",
                path.display()
            );
            Ok(())
        }
        Err(e) if e.kind() == ErrorKind::AlreadyExists => Err(anyhow!(
            "A configuration file already exists at {}",
            path.display()
        )),
        Err(e) => Err(Error::new(e).context(format!(
            "Failed to create default configuration file at {}",
            path.display()
        ))),
    }
}

#[derive(Debug, Deserialize)]
pub struct Configuration {
    pub submission: Submission,
    #[serde(default)]
    pub mpd: Mpd,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Token {
    Inline { token: String },
    File { token_file: PathBuf },
}

impl Token {
    pub fn value(&self) -> &str {
        if let Token::Inline { token } = self {
            token.trim()
        } else {
            panic!("Token value was not determined while parsing.");
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Submission {
    #[serde(flatten)]
    pub token: Token,
    #[serde(default = "default_api_url")]
    pub api_url: String,
    #[serde(default = "genres_as_folksonomy")]
    pub genres_as_folksonomy: bool,
    #[serde(default)]
    pub genre_separator: Option<char>,
    #[serde(default = "default_cache")]
    pub enable_cache: bool,
    #[serde(default)]
    pub cache_file: Option<PathBuf>,
}

fn default_cache() -> bool {
    true
}

fn default_api_url() -> String {
    String::from("https://api.listenbrainz.org")
}

fn genres_as_folksonomy() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Mpd {
    pub address: String,
    pub password: Option<String>,
    pub password_file: Option<PathBuf>,
}

impl Default for Mpd {
    fn default() -> Self {
        Mpd {
            address: String::from("127.0.0.1:6600"),
            password: None,
            password_file: None,
        }
    }
}
