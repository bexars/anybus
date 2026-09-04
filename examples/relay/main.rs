use clap::Parser;

use crate::cli::Args;
#[path = "../shared/mod.rs"]
mod shared;

pub(crate) mod cli;
// pub(crate) mod config;

use anybus::AnyBusConfig;

#[tokio::main]
async fn main() {
    // tracing_subscriber::fmt::fmt()
    //     .with_max_level(tracing_subscriber::filter::LevelFilter::TRACE)
    //     .init();
    let _log_guard = shared::logging::init_tracing("anybus-relay");

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
    let handle = anybus.handle().clone();
    let mut watch = handle
        .get_anybus_status_receiver()
        .await
        .expect("Failed to get AnyBus status receiver");

    loop {
        let status = watch.recv().await;
        println!("AnyBus status: {:?}", status);

        match status {
            Ok(_status) => {}
            Err(err) => match err {
                anybus::ReceiveError::ConnectionClosed => todo!(),
                anybus::ReceiveError::RegistrationFailed(_) => todo!(),
                anybus::ReceiveError::DeserializationError(_payload) => todo!(),
                anybus::ReceiveError::Shutdown => {
                    println!("Shutting down");
                    break;
                }
                anybus::ReceiveError::RpcNoReplyTo => todo!(),
            },
        }
    }
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    println!("Goodbye.");
}
