use std::time::Duration;

use async_bincode::{AsyncDestination, tokio::AsyncBincodeStream};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};

use crate::tokio;
use crate::tokio::sync::oneshot;
use crate::tokio::time::{Instant, timeout};
use tokio::io::DuplexStream;
use tokio::select;

use crate::{
    Handle, Realm,
    messages::NodeMessage,
    peers::common::{Heartbeat, Peer},
    routing::{ConnectionIdCounter, Cost, NodeId, RealmList},
};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(30);

type DummyStream = AsyncBincodeStream<DuplexStream, DummyMessage, DummyMessage, AsyncDestination>;

#[derive(Debug, Serialize, Deserialize)]
enum DummyMessage {
    Hello(NodeId),
    NodeMsg(NodeMessage),
    Ping(u64),
    Pong(u64),
    Shutdown,
}

/// How [`DummyPeerKill`] closes the dummy link.
#[derive(Debug)]
pub enum DummyPeerStop {
    /// Send a shutdown frame, then close this half of the link.
    Shutdown,
    /// Stop sending and answering. The duplex stays open so the other side
    /// reaches its heartbeat timeout instead of seeing a dropped stream.
    Silence,
    /// Drop this half of the duplex without a shutdown frame.
    Drop,
}

/// Oneshot that closes the link started by [`crate::AnyBus::new_dummy_peer`].
#[derive(Debug)]
pub struct DummyPeerKill {
    stop: oneshot::Sender<DummyPeerStop>,
}

impl DummyPeerKill {
    pub(crate) fn new(stop: oneshot::Sender<DummyPeerStop>) -> Self {
        Self { stop }
    }

    /// Send a shutdown frame and close this half of the link.
    pub fn shutdown(self) -> Result<(), DummyPeerStop> {
        self.stop.send(DummyPeerStop::Shutdown)
    }

    /// Stop sending and answering so the other side's heartbeat expires.
    pub fn silence(self) -> Result<(), DummyPeerStop> {
        self.stop.send(DummyPeerStop::Silence)
    }

    /// Drop this half of the duplex without a shutdown frame.
    pub fn drop_link(self) -> Result<(), DummyPeerStop> {
        self.stop.send(DummyPeerStop::Drop)
    }
}

enum Event {
    Remote(DummyMessage),
    Local(NodeMessage),
    Tick,
    Closed,
    Stop(DummyPeerStop),
    Ignore,
}

pub(crate) async fn run(
    stream: DuplexStream,
    handle: Handle,
    our_id: NodeId,
    connection_counter: ConnectionIdCounter,
    cost: Cost,
    realms: RealmList,
    mut kill: oneshot::Receiver<DummyPeerStop>,
) {
    let mut stream: DummyStream = AsyncBincodeStream::from(stream).for_async();
    let mut kill_open = true;
    if let Err(err) = stream.send(DummyMessage::Hello(our_id)).await {
        tracing::debug!("dummy peer failed to send hello: {err}");
        return;
    }

    let peer_id = loop {
        let hello = select! {
            msg = timeout(HANDSHAKE_TIMEOUT, stream.next()) => msg,
            kind = &mut kill, if kill_open => {
                kill_open = false;
                match kind {
                    Ok(kind) => {
                        stop_link(kind, &mut stream, None).await;
                        return;
                    }
                    Err(_) => continue,
                }
            },
        };
        break match hello {
            Ok(Some(Ok(DummyMessage::Hello(id)))) => id,
            Ok(Some(Ok(other))) => {
                tracing::debug!("dummy peer expected Hello, got {other:?}");
                return;
            }
            Ok(Some(Err(err))) => {
                tracing::debug!("dummy peer hello failed: {err}");
                return;
            }
            Ok(None) | Err(_) => {
                tracing::debug!("dummy peer hello timed out");
                return;
            }
        };
    };

    let connection_id = connection_counter.next();
    let mut peer = Peer::register_peer(
        peer_id,
        our_id,
        handle,
        Realm::Userspace,
        connection_id,
        cost,
        realms,
        "",
    );
    let mut hb = Heartbeat::new(Instant::now(), HEARTBEAT_INTERVAL, HEARTBEAT_TIMEOUT);
    tracing::info!("dummy peer connected to {peer_id}");

    loop {
        let event = select! {
            msg = stream.next() => match msg {
                Some(Ok(message)) => Event::Remote(message),
                Some(Err(err)) => {
                    tracing::warn!("dummy peer read failed: {err}");
                    Event::Closed
                }
                None => Event::Closed,
            },
            msg = peer.recv() => match msg {
                Some(message) => Event::Local(message),
                None => Event::Closed,
            },
            _ = tokio::time::sleep_until(hb.next_deadline()) => Event::Tick,
            kind = &mut kill, if kill_open => {
                kill_open = false;
                match kind {
                    Ok(kind) => Event::Stop(kind),
                    Err(_) => Event::Ignore,
                }
            },
        };

        match event {
            Event::Remote(DummyMessage::NodeMsg(message)) => {
                hb.on_rx(Instant::now());
                peer.handle_node_message(message);
            }
            Event::Remote(DummyMessage::Ping(token)) => {
                hb.on_rx(Instant::now());
                if stream.send(DummyMessage::Pong(token)).await.is_err() {
                    break;
                }
            }
            Event::Remote(DummyMessage::Pong(_)) => {
                hb.on_rx(Instant::now());
            }
            Event::Remote(DummyMessage::Hello(_)) => {}
            Event::Remote(DummyMessage::Shutdown) => {
                tracing::debug!("dummy peer {peer_id} received shutdown");
                peer.unregister();
                stream.close().await.ok();
                return;
            }
            Event::Local(message) => {
                if stream.send(DummyMessage::NodeMsg(message)).await.is_err() {
                    break;
                }
            }
            Event::Tick => {
                let now = Instant::now();
                if hb.timed_out(now) {
                    tracing::warn!("dummy peer {peer_id} heartbeat timed out");
                    break;
                }
                if hb.ping_due(now) {
                    let token = hb.take_ping_token(now);
                    if stream.send(DummyMessage::Ping(token)).await.is_err() {
                        break;
                    }
                }
            }
            Event::Stop(DummyPeerStop::Drop) => {
                peer.unregister();
                return;
            }
            Event::Stop(kind) => {
                stop_link(kind, &mut stream, Some(&mut peer)).await;
                return;
            }
            Event::Closed => break,
            Event::Ignore => {}
        }
    }

    peer.unregister();
    stream.close().await.ok();
}

/// Close this half of the link.
///
/// `Silence` keeps the duplex open and discards traffic until the other side
/// closes it. `Drop` is handled by the caller so this stream is not shut down
/// cleanly here.
async fn stop_link(kind: DummyPeerStop, stream: &mut DummyStream, mut peer: Option<&mut Peer>) {
    match kind {
        DummyPeerStop::Shutdown => {
            stream.send(DummyMessage::Shutdown).await.ok();
            if let Some(peer) = peer.as_mut() {
                peer.unregister();
            }
            stream.close().await.ok();
        }
        DummyPeerStop::Silence => {
            if let Some(peer) = peer.as_mut() {
                loop {
                    select! {
                        msg = stream.next() => match msg {
                            Some(Ok(_)) => {}
                            Some(Err(_)) | None => break,
                        },
                        msg = peer.recv() => match msg {
                            Some(_) => {}
                            None => break,
                        },
                    }
                }
                peer.unregister();
            } else {
                while stream.next().await.is_some() {}
            }
            stream.close().await.ok();
        }
        DummyPeerStop::Drop => {
            if let Some(peer) = peer.as_mut() {
                peer.unregister();
            }
        }
    }
}
