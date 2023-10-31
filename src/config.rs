use std::{
    env,
    fs::{self, File},
    io::{self, ErrorKind, Write},
    path::PathBuf,
};

use anyhow::{anyhow, bail, Context, Error, Result};
use serde::Deserialize;
use tracing::debug;

/// The default configuration file.
pub const DEFAULT: &[u8] = include_str!("../config.toml.sample").as_bytes();

/// Parsed & validated configuration.
#[derive(Debug)]
pub struct Configuration {
    /// The user token
    pub token: String,
    /// The submission API URL (without a trailing slash)
    pub api_url: String,
    /// The MPD host
    pub mpd_host: String,
    /// The MPD port
    pub mpd_port: u16,
    /// The MPD server password
    pub mpd_password: Option<String>,
    /// Whether to enable caching failed submissions
    pub enable_cache: bool,
    /// Path to the file used for caching listens
    pub cache_file: PathBuf,
    /// Whether to submit genre tags
    pub submit_genres_as_folksonomy: bool,
    /// Separator character for single-value genre tags
    pub genre_separator: Option<char>,
}

fn default_path() -> PathBuf {
    let mut p = dirs::config_dir().expect("no config directory on this platform");
    p.push(concat!(env!("CARGO_PKG_NAME"), "/config.toml"));
    p
}

pub fn load(path: Option<PathBuf>) -> Result<Configuration> {
    let path = &path.unwrap_or_else(default_path);

    debug!(?path, "loading configuration file");

    // Load configuration file or the default base config
    let mut config = match fs::read_to_string(path) {
        Ok(c) => {
            // Configuration file exists, parse it
            toml::from_str(&c).with_context(|| {
                format!("Failed to parse configuration file at {}", path.display())
            })?
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // Configuration file was not found, use the default config
            debug!("configuration file not found");
            RawConfiguration::default()
        }
        Err(e) => {
            return Err(anyhow::Error::new(e).context(format!(
                "Failed to read configuration file at {}",
                path.display()
            )));
        }
    };

    // Check if both `submission.token` and `submission.token_file` are given
    if config.submission.token.is_some() && config.submission.token_file.is_some() {
        bail!("`submission.token_file` cannot be set when `submission.token` is also set");
    }

    // Check if both `mpd.password` and `mpd.password_file` are given
    if config.mpd.password.is_some() && config.mpd.password_file.is_some() {
        bail!("`mpd.password_file` cannot be set when `mpd.password` is also set");
    }

    // The token can be specified using the LISTENBRAINZ_TOKEN environment variable
    if let Some(token) = env_var("LISTENBRAINZ_TOKEN")? {
        debug!("found token in environment variable");
        config.submission.token = Some(token);
    }

    // Read `submission.token_file` if the token isn't known by this point
    if let (None, Some(token_file)) = (&config.submission.token, config.submission.token_file) {
        debug!(?token_file, "loading token from `submission.token_file`");
        let token = fs::read_to_string(&token_file).with_context(|| {
            format!(
                "Failed to read `submission.token_file` at {}",
                token_file.display()
            )
        })?;
        config.submission.token = Some(token.trim().to_owned());
    }

    // The MPD address and password can be specified in the MPD_HOST and MPD_PORT
    // environment variables (compatible with tools like MPC)
    if let Some(mpd_host) = env_var("MPD_HOST")? {
        // The syntax of the value is `password@host`, with the password part
        // optional
        if let Some((password, host)) = mpd_host.split_once('@') {
            debug!("found MPD_HOST environment variable with host and password");
            config.mpd.address = Some(host.to_owned());
            config.mpd.password = Some(password.to_owned());
        } else {
            debug!("found MPD_HOST environment variable with only host");
            config.mpd.address = Some(mpd_host);
        }
    }

    // Read `mpd.password_file` if the password isn't known at this point
    if let (None, Some(password_file)) = (&config.mpd.password, config.mpd.password_file) {
        debug!(
            ?password_file,
            "loading MPD password from `mpd.password_file"
        );
        let password = fs::read_to_string(&password_file).with_context(|| {
            format!(
                "Failed to read `mpd.password_file` at {}",
                password_file.display()
            )
        })?;
        config.mpd.password = Some(password.trim().to_owned());
    }

    let token = match config.submission.token {
        Some(token) if token.is_empty() => bail!("ListenBrainz token value cannot be empty"),
        Some(token) => token,
        None => bail!("Could not find ListenBrainz token in configuration or environment"),
    };

    // Remove trailing slashes from configured API URL or fall back to default
    let api_url = if let Some(url) = config.submission.api_url {
        let url = url.trim_end_matches('/');
        if url.is_empty() {
            bail!("`submission.api_url` cannot be empty");
        }

        url.to_owned()
    } else {
        String::from("https://api.listenbrainz.org")
    };

    // Determine the MPD port from either the configuration address string or the
    // MPD_PORT environment variable, or fall back to the default port
    let mpd_port = if let Some(port) = env_var("MPD_PORT")? {
        debug!("found MPD_PORT environment variable");
        port.parse()
            .with_context(|| format!("Invalid MPD_PORT value: {port:?}"))?
    } else if let Some((h, p)) = config.mpd.address.as_deref().and_then(split_address_port) {
        let port = p
            .parse()
            .with_context(|| format!("Invalid port in `mpd.address`: {p:?}"))?;
        // Remove the port from the host string
        config.mpd.address = Some(h.to_owned());
        port
    } else {
        // Default port
        6600
    };

    let mpd_host = match config.mpd.address {
        Some(host) if host.is_empty() => bail!("MPD host cannot be empty"),
        Some(host) => host,
        None => String::from("localhost"),
    };

    Ok(Configuration {
        token,
        api_url,
        mpd_host,
        mpd_port,
        mpd_password: config.mpd.password,
        enable_cache: config.submission.enable_cache,
        cache_file: dirs::data_local_dir()
            .expect("No state/cache directory")
            .join("listenbrainz-mpd-cache.sqlite3"),
        submit_genres_as_folksonomy: config.submission.genres_as_folksonomy,
        genre_separator: config.submission.genre_separator,
    })
}

/// Parse the port from an address string of the form `address:port`. Returns
/// the host portion and the port portion if found, None otherwise.
fn split_address_port(address: &str) -> Option<(&str, &str)> {
    if address.starts_with('/') {
        // Unix socket path, don't attempt to parse
        return None;
    }

    address.rsplit_once(':')
}

pub fn create_default_config() -> Result<()> {
    let path = default_path();

    // Create directories if necessary
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)
            .with_context(|| format!("Failed to create directories: {}", p.display()))?;
    }

    // Create the actual config file and write the contents into it, but only if it
    // does not already exist
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

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawConfiguration {
    submission: RawSubmissionConfig,
    mpd: RawMpdConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct RawSubmissionConfig {
    token: Option<String>,
    token_file: Option<PathBuf>,
    api_url: Option<String>,
    genres_as_folksonomy: bool,
    genre_separator: Option<char>,
    enable_cache: bool,
    cache_file: Option<PathBuf>,
}

impl Default for RawSubmissionConfig {
    fn default() -> Self {
        RawSubmissionConfig {
            token: None,
            token_file: None,
            api_url: None,
            genres_as_folksonomy: true,
            genre_separator: None,
            enable_cache: true,
            cache_file: None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawMpdConfig {
    address: Option<String>,
    password: Option<String>,
    password_file: Option<PathBuf>,
}

/// Load the value of the environment variable with the given name.
fn env_var(name: &str) -> Result<Option<String>> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(other) => Err(anyhow::Error::new(other)
            .context(format!("Failed to read environment variable {name}"))),
    }
}
