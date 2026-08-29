use clap::Parser;

use crate::cli::Args;

pub(crate) mod cli;
// pub(crate) mod config;

pub(crate) use anybus::config::load_config;

#[tokio::main]
async fn main() {
    let args = Args::parse();

    if let Some(config) = &args.config {
        let bus_config = load_config(config).expect("Failed to load config");
        println!("Config file: {}", config.display());
        println!("Config:");
        println!("{:?}", bus_config);
    }

    for ws in &args.ws {
        println!("Websocket: {ws}");
    }
    if let Some(ws_server) = &args.ws_server {
        println!("Websocket server: {ws_server}");
    }
    if let Some(ws_cert) = &args.ws_cert {
        println!("Websocket server certificate: {}", ws_cert.display());
    }
    if let Some(ws_key) = &args.ws_key {
        println!("Websocket server key: {}", ws_key.display());
    }
}
