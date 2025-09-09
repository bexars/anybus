use std::net::{IpAddr, SocketAddr};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tracing::error;
use uuid::Uuid;

use crate::{messages::BusControlMsg, spawn};

pub(super) mod ws_manager;
mod ws_peer;

enum WsControl {}

#[derive(Debug)]
enum WsCommand {
    NewTcpStream(tokio::net::TcpStream, SocketAddr),
}

enum WsMessage {
    Hello(Uuid),
}

#[derive(Debug, thiserror::Error)]
enum WsError {
    #[error("Error binding to address {}", .0)]
    BindFailure(SocketAddr),
}

/// Options for the WebSocket listener
#[derive(Debug, Clone)]
pub struct WsListenerOptions {
    pub addr: IpAddr,
    pub port: u16,
    pub use_tls: bool,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
}

impl Default for WsListenerOptions {
    fn default() -> Self {
        Self {
            addr: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            port: 8080,
            use_tls: false,
            cert_path: None,
            key_path: None,
        }
    }
}

async fn create_listener(
    ws_listener_options: WsListenerOptions,
    ws_command: tokio::sync::mpsc::UnboundedSender<WsCommand>,
    bus_control: tokio::sync::watch::Receiver<BusControlMsg>,
) -> Result<(), WsError> {
    // Create the listener here
    //
    let sock_addr = std::net::SocketAddr::new(ws_listener_options.addr, ws_listener_options.port);
    let listener = tokio::net::TcpListener::bind(sock_addr)
        .await
        .map_err(|e| {
            error!("Failed to bind to address {}: {}", sock_addr, e);
            WsError::BindFailure(sock_addr)
        })?;
    spawn(run_ws_listener(listener, ws_command, bus_control));
    Ok(())
}

async fn run_ws_listener(
    listener: tokio::net::TcpListener,
    ws_command: UnboundedSender<WsCommand>,
    mut bus_control: tokio::sync::watch::Receiver<BusControlMsg>,
) {
    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, addr)) => {
                        // Handle the new connection

                        tracing::info!("Accepted connection from {}", addr);
                        ws_command.send(WsCommand::NewTcpStream(stream,addr)).ok();
                    }
                    Err(e) => {
                        error!("Failed to accept connection: {}", e);
                    }
                }
            }
            Ok(msg) = bus_control.changed() => {
                let msg = *bus_control.borrow();
                match msg {
                    BusControlMsg::Shutdown => {
                        break;
                    }
                    _ => {}
                }
            }

        }
    }
}
