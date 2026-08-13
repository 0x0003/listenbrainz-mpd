#[cfg(unix)]
use std::os::unix::{
    fs::{DirBuilderExt, OpenOptionsExt},
    net::SocketAddr as StdSocketAddr,
};
use std::{
    env,
    fs::{self, File},
    io::{self, ErrorKind, Write},
    net::{SocketAddr, ToSocketAddrs},
    num::NonZero,
    path::PathBuf,
};

use anyhow::{Context, Error, Result, anyhow, bail};
use serde::Deserialize;
#[cfg(unix)]
use tokio::net::unix;
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
    pub mpd_address: MpdAddress,
    /// The MPD server password
    pub mpd_password: Option<String>,
    /// Whether to enable caching failed submissions
    pub enable_cache: bool,
    /// Whether to submit genre tags
    pub submit_genres_as_folksonomy: bool,
    /// Separator character for single-value genre tags
    pub genre_separator: Option<char>,
    /// Path to the file used for caching listens
    pub cache_file: Option<PathBuf>,
}

#[derive(Debug)]
pub enum MpdAddress {
    Tcp {
        raw_address: String,
        resolved: Vec<SocketAddr>,
    },
    #[cfg(unix)]
    Unix(unix::SocketAddr),
}

fn default_path() -> PathBuf {
    let mut p = dirs::config_dir().expect("no config directory on this platform");
    p.push(concat!(env!("CARGO_PKG_NAME"), "/config.toml"));
    p
}

pub fn load(path: Option<PathBuf>) -> Result<Configuration> {
    let path_from_cli = path.is_some();
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
        Err(e) if e.kind() == io::ErrorKind::NotFound && !path_from_cli => {
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

    // The token can be specified using the LISTENBRAINZ_TOKEN environment variable,
    // which takes precedence over the configuration file
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

    let token = match config.submission.token {
        Some(token) if token.is_empty() => bail!("ListenBrainz token value cannot be empty"),
        Some(token) => token,
        None => bail!("Could not find ListenBrainz token in configuration or environment"),
    };

    // Determine the host and optionally the connection password from the MPD_HOST
    // environment variable (syntax compatible with mpc)
    if let Some(mpd_host) = env_var("MPD_HOST")? {
        // The syntax of the value is `password@host`, with the password part
        // optional. Note that if an abstract socket is being used (linux only), the
        // format will be `password@@abstract_socket`.
        if let Some((password, host)) = mpd_host.split_once('@')
            && !password.is_empty()
        {
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

    // If the address isn't set at this point, assume default
    let host = config
        .mpd
        .address
        .unwrap_or_else(|| String::from("localhost"));

    // Parse the MPD_PORT environment variable, which may override the port from the
    // configuration
    let mpd_port = env_var("MPD_PORT")?
        .map(|p| {
            p.parse::<NonZero<u16>>()
                .with_context(|| format!("Invalid MPD_PORT value: {p:?}"))
        })
        .transpose()?;

    // Determine the kind of MPD address
    let mpd_address = if host.starts_with('/') {
        // Unix socket
        cfg_select! {
            unix => {
                let addr = StdSocketAddr::from_pathname(&host)
                    .with_context(|| format!("Invalid Unix socket address: {host:?}"))?;
                MpdAddress::Unix(addr.into())
            }
            _ => bail!("Unix sockets are not supported on this platform"),
        }
    } else if host.starts_with('@') {
        // Abstract unix socket
        cfg_select! {
            target_os = "linux" => {
                use std::os::linux::net::SocketAddrExt;
                let addr = StdSocketAddr::from_abstract_name(&host[1..])
                    .with_context(|| format!("Invalid abstract socket address: {host:?}"))?;
                MpdAddress::Unix(addr.into())
            }
            _ => bail!("Abstract sockets (starting with '@') are only supported on Linux"),
        }
    } else {
        // TCP, as a hostname or bare IP address
        let mut resolved = resolve_mpd_host(&host)
            .with_context(|| format!("Failed to parse or resolve hostname: {host:?}"))?;

        // Override the port from the config with the env var if set
        if let Some(p) = mpd_port.map(NonZero::get) {
            resolved.iter_mut().for_each(|addr| addr.set_port(p));
        }

        MpdAddress::Tcp {
            raw_address: host,
            resolved,
        }
    };

    Ok(Configuration {
        token,
        api_url,
        mpd_address,
        mpd_password: config.mpd.password,
        enable_cache: config.submission.enable_cache,
        cache_file: config.submission.cache_file,
        submit_genres_as_folksonomy: config.submission.genres_as_folksonomy,
        genre_separator: config.submission.genre_separator,
    })
}

fn resolve_mpd_host(host: &str) -> Result<Vec<SocketAddr>> {
    // Try to parse the host, first as an IP address or hostname without a port or,
    // if that fails, as an IP address or hostname with a port included
    let res = (host, 6600)
        .to_socket_addrs()
        .or_else(|error| {
            debug!(%error, "failed to resolve as address without port");
            host.to_socket_addrs()
        })?
        .collect::<Vec<_>>();

    if res.is_empty() {
        bail!("Could not resolve to any address");
    }

    Ok(res)
}

pub fn create_default_config() -> Result<()> {
    let path = default_path();

    // Create directories if necessary
    if let Some(p) = path.parent() {
        let mut builder = fs::DirBuilder::new();

        #[cfg(unix)]
        builder.mode(0o700);

        builder
            .recursive(true)
            .create(p)
            .with_context(|| format!("Failed to create config directories at: {}", p.display()))?;
    }

    // Create the actual config file and write the contents into it, but only if it
    // does not already exist
    let mut file_options = File::options();

    #[cfg(unix)]
    file_options.mode(0o600);

    match file_options.write(true).create_new(true).open(&path) {
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
        Ok(value) if value.is_empty() => Err(anyhow!(
            "Environment variable {name} must not be empty if set"
        )),
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(other) => Err(anyhow::Error::new(other)
            .context(format!("Failed to read environment variable {name}"))),
    }
}
