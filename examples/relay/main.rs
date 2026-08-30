use clap::Parser;
use std::io::stdin;

use crate::cli::Args;

pub(crate) mod cli;
// pub(crate) mod config;

use anybus::AnyBusConfig;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::fmt()
        .with_max_level(tracing_subscriber::filter::LevelFilter::DEBUG)
        .init();

    let args = Args::parse();

    let config = if let Some(config) = &args.config {
        let bus_config = AnyBusConfig::load_config(config).expect("Failed to load config");
        println!("Config file: {}", config.display());
        println!("Config:");
        println!("{:#?}", bus_config);
        bus_config
    } else {
        println!("No config file specified, using default config");
        AnyBusConfig::default()
    };
    let mut anybus = config.init();
    anybus.run();
    stdin()
        .read_line(&mut String::new())
        .expect("Failed to read from stdin");
    anybus.shutdown();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
}
