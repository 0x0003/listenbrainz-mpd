mod config;

use std::{net::SocketAddr, path::Path};

use anyhow::{Context, Result};
use mpd_client::{state_changes::StateChanges, Client};
use tokio::net::{TcpStream, UnixStream};
use tracing::debug;
use tracing_subscriber::EnvFilter;

use crate::config::MpdConnection;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_env("LISTENBRAINZ_MPD_LOG"))
        .init();

    let config = config::load(Path::new("./config.toml"))?;

    let (mpd_client, state_changes) = connect(&config.mpd).await?;

    Ok(())
}

async fn connect(mpd_config: &config::Mpd) -> Result<(Client, StateChanges)> {
    match &mpd_config.connection {
        MpdConnection::Tcp { ip, port } => {
            let address = SocketAddr::new(*ip, port.get());
            connect_tcp(address)
                .await
                .with_context(|| format!("failed to connect to {}", address))
        }
        MpdConnection::UnixSocket { unix } => connect_unix(unix)
            .await
            .with_context(|| format!("failed to connect via Unix socket at {}", unix.display())),
    }
}

async fn connect_tcp(address: SocketAddr) -> Result<(Client, StateChanges)> {
    debug!(?address, "connecting via TCP");
    let socket = TcpStream::connect(address).await?;
    Client::connect(socket).await.map_err(Into::into)
}

async fn connect_unix(path: &Path) -> Result<(Client, StateChanges)> {
    debug!(?path, "connecting via Unix socket");
    let socket = UnixStream::connect(path).await?;
    Client::connect(socket).await.map_err(Into::into)
}
