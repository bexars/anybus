use std::sync::Arc;

use async_trait::async_trait;
use tokio::{
    net::unix::SocketAddr,
    sync::{
        RwLock,
        mpsc::{UnboundedReceiver, UnboundedSender},
        watch,
    },
};
use tracing::{debug, error};
use uuid::Uuid;

use crate::{
    Handle,
    messages::BusControlMsg,
    peers::ws::{WsCommand, WsControl, WsError, WsListenerOptions, create_listener},
    routing::NodeId,
};

/// Helper function to box a State and return it as an Option
fn b<T: State + 'static>(thing: T) -> Option<Box<dyn State>> {
    Some(Box::new(thing))
}

#[derive(Debug)]
pub(crate) struct WebsocketManager {
    handle: Handle,
    bus_control: watch::Receiver<BusControlMsg>,
    peers: Vec<(Uuid, UnboundedSender<WsControl>)>,
    tx: UnboundedSender<WsCommand>,
    rx: UnboundedReceiver<WsCommand>,
    node_id: NodeId,
    ws_listener_options: Option<WsListenerOptions>,
    listener: Option<tokio::net::TcpListener>,
}

impl WebsocketManager {
    pub(crate) fn new(
        node_id: NodeId,
        handle: Handle,
        bus_control: watch::Receiver<BusControlMsg>,
        ws_listener_options: WsListenerOptions,
    ) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            handle,
            bus_control,
            peers: Vec::new(),
            tx,
            rx,
            node_id,
            listener: None,
            ws_listener_options: Some(ws_listener_options),
        }
    }

    pub(crate) async fn start(mut self) {
        let mut state = Some(Box::new(Start {}) as Box<dyn State>);
        while let Some(old_state) = state.take() {
            debug!("Entering: {:?}", &old_state);
            state = old_state.next(&mut self).await;
        }
    }
}

#[async_trait]
trait State: Send + std::fmt::Debug {
    async fn next(self: Box<Self>, state: &mut WebsocketManager) -> Option<Box<dyn State>>;
}

#[derive(Debug)]
struct Start;

#[derive(Debug)]
struct Listen;

#[derive(Debug)]
struct HandleCommand(WsCommand);

#[derive(Debug)]
struct HandShake {
    stream: tokio::net::TcpStream,
    addr: core::net::SocketAddr,
}

#[async_trait]
impl State for Start {
    async fn next(self: Box<Self>, state: &mut WebsocketManager) -> Option<Box<dyn State>> {
        if let Some(ws_options) = state.ws_listener_options.take() {
            match create_listener(ws_options, state.tx.clone(), state.bus_control.clone()).await {
                Ok(()) => b(Listen {}),
                Err(e) => {
                    error!("Failed to create WebSocket listener: {}", e);
                    None
                }
            }
        } else {
            error!("WebSocket listener options not provided");
            None
        }
    }
}

#[async_trait]
impl State for Listen {
    async fn next(self: Box<Self>, state: &mut WebsocketManager) -> Option<Box<dyn State>> {
        tokio::select! {
            Some(cmd) = state.rx.recv() => {
                b(HandleCommand(cmd))
                // Handle incoming commands
            }
            Ok(msg) = state.bus_control.changed() => {
                match *state.bus_control.borrow() {
                    BusControlMsg::Shutdown => {
                        debug!("WebSocket Manager shutting down");
                         None
                    }
                    _ => {
                        b(Listen)
                    }
                }
            }
            else => {
                error!("WebSocket Manager closed.  Shutting down Websockets.");
                None
            }
        }
    }
}

#[async_trait]
impl State for HandleCommand {
    async fn next(self: Box<Self>, state: &mut WebsocketManager) -> Option<Box<dyn State>> {
        match self.0 {
            WsCommand::NewTcpStream(stream, addr) => b(HandShake { stream, addr }),
        }
    }
}

#[async_trait]
impl State for HandShake {
    async fn next(self: Box<Self>, state: &mut WebsocketManager) -> Option<Box<dyn State>> {
        // Perform the WebSocket handshake here

        debug!("Performing handshake with {}", self.addr);
        let stream_result = tokio_tungstenite::accept_async(self.stream).await;
        match stream_result {
            Ok(ws_stream) => {
                debug!("WebSocket connection established with {}", self.addr);
                // Here you would typically spawn a new task to handle the WebSocket connection
                // For simplicity, we just log and return to listening state
                b(Listen {})
            }
            Err(e) => {
                error!("WebSocket handshake failed with {}: {}", self.addr, e);
                b(Listen {})
            }
        }
    }
}
