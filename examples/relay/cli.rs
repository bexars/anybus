use std::{net::SocketAddr, path::PathBuf};

use clap::Parser;
use url::Url;

#[derive(Debug, Parser)]
pub(crate) struct Args {
    #[arg(long = "ws", value_parser = parse_ws_url, help = "WebSocket peer URL (ws:// or wss://)")]
    pub(crate) ws: Vec<Url>,
    #[arg(long = "ws_server", value_parser = clap::value_parser!(SocketAddr), help = "Bind address for the websocket server")]
    pub(crate) ws_server: Option<SocketAddr>,
    #[arg(long, value_parser = file_exists, requires = "ws_server", help ="PEM file for the server certificate")]
    pub(crate) ws_cert: Option<PathBuf>,
    #[arg(long, value_parser = file_exists, requires = "ws_server", help = "PEM file for the certificate key")]
    pub(crate) ws_key: Option<PathBuf>,
    #[arg(short = 'c', long, value_parser = file_exists, help = "Path to the Anybus configuration file",
        conflicts_with_all = ["ws", "ws_server"])]
    pub(crate) config: Option<PathBuf>,
}

fn parse_ws_url(s: &str) -> Result<Url, String> {
    let url = Url::parse(s).map_err(|e| e.to_string())?;
    match url.scheme() {
        "ws" | "wss" => Ok(url),
        other => Err(format!("expected ws:// or wss://, got `{other}`")),
    }
}

fn file_exists(s: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(s);
    if !path.exists() {
        return Err(format!("file does not exist: {}", path.display()));
    }
    if !path.is_file() {
        return Err(format!("not a file: {}", path.display()));
    }
    Ok(path)
}
