use std::collections::VecDeque;

use futures::{SinkExt, StreamExt};
use tokio::{
    // io::{AsyncRead, AsyncWrite},
    select,
    sync::{
        mpsc::{UnboundedReceiver, UnboundedSender},
        watch,
    },
};

// #[cfg(not(feature = "wasm_ws"))]
// use tokio_tungstenite::{WebSocketStream, tungstenite::Message};
// #[cfg(feature = "wasm_ws")]
// use tokio_tungstenite_wasm::{Message, WebSocketStream};

use tracing::{debug, error, trace};

use crate::{
    messages::{BrokerMsg, BusControlMsg, NodeMessage},
    peers::{
        Peer,
        ws::{WebSockStream, WsCommand, WsControl, WsMessage},
    },
    routing::{NodeId, WirePacket},
};

#[derive(Debug)]
enum OutMessage {
    WsCommand(WsCommand),
    WsMessage(WsMessage),
    BrokerMessage(BrokerMsg),
    ClosePeer,
    ForwardPacket(WirePacket),
}

#[derive(Debug)]
pub enum InMessage {
    WsControl(WsControl),
    WsMessage(WsMessage),
    NodeMessage(NodeMessage),
    WsPeerClosed,
    Shutdown,
    Unknown,
}

enum State {
    Running,
    Shutdown,
}

#[allow(unused)]
struct WsPeer {
    output: VecDeque<OutMessage>,
    state: State,
    peer_id: NodeId,
    our_id: NodeId,
}

impl WsPeer {
    fn new(our_id: NodeId, peer_id: NodeId) -> Self {
        trace!(
            "Creating new WsPeer with our_id: {} and peer_id: {}",
            our_id, peer_id
        );
        Self {
            peer_id,
            our_id,
            output: VecDeque::new(),
            state: State::Running,
        }
    }

    fn poll_outgoing(&mut self) -> Option<OutMessage> {
        self.output.pop_front()
    }

    fn handle_input(&mut self, in_message: InMessage) {
        trace!("WsPeer handling input: {:?}", &in_message);
        match in_message {
            InMessage::WsControl(cmd) => {
                match cmd {
                    WsControl::Shutdown => {
                        self.output
                            .push_back(OutMessage::WsMessage(WsMessage::CloseConnection));
                        self.output.push_back(OutMessage::ClosePeer);
                    }
                }
                // Handle WsControl commands
            }
            InMessage::WsMessage(msg) => {
                // Handle incoming WsMessage
                match msg {
                    WsMessage::Hello(uuid) => {
                        error!("Received unexpected Hello from peer {}", uuid);
                        // Protocol violation, initiate shutdown
                        self.output
                            .push_back(OutMessage::WsMessage(WsMessage::CloseConnection));
                        self.output.push_back(OutMessage::BrokerMessage(
                            BrokerMsg::UnRegisterPeer(self.peer_id),
                        ));
                        self.output.push_back(OutMessage::ClosePeer);
                        self.output
                            .push_back(OutMessage::WsCommand(WsCommand::PeerClosed(self.peer_id)));
                        self.state = State::Shutdown;
                    }
                    WsMessage::CloseConnection => {
                        self.output.push_back(OutMessage::BrokerMessage(
                            BrokerMsg::UnRegisterPeer(self.peer_id),
                        ));
                        self.output.push_back(OutMessage::ClosePeer);
                        self.output
                            .push_back(OutMessage::WsCommand(WsCommand::PeerClosed(self.peer_id)));
                        self.state = State::Shutdown;
                    }
                    WsMessage::Packet(wire_packet) => {
                        trace!("Forwarding packet from WS peer {} to broker", self.peer_id);
                        self.output
                            .push_back(OutMessage::ForwardPacket(wire_packet));
                    }
                    WsMessage::Advertise(hash_set) => {
                        self.output.push_back(OutMessage::BrokerMessage(
                            BrokerMsg::AddPeerEndpoints(self.peer_id, hash_set),
                        ));
                    }
                    WsMessage::Withdraw(hash_set) => {
                        self.output.push_back(OutMessage::BrokerMessage(
                            BrokerMsg::RemovePeerEndpoints(self.peer_id, hash_set),
                        ));
                    }
                }
            }
            InMessage::NodeMessage(msg) => {
                // Handle NodeMessage
                match msg {
                    NodeMessage::WirePacket(wire_packet) => {
                        self.output
                            .push_back(OutMessage::WsMessage(WsMessage::Packet(wire_packet)));
                    }
                    NodeMessage::Close => {
                        self.output.push_back(OutMessage::ClosePeer);

                        self.output.push_back(OutMessage::BrokerMessage(
                            BrokerMsg::UnRegisterPeer(self.peer_id),
                        ));
                        self.output
                            .push_back(OutMessage::WsCommand(WsCommand::PeerClosed(self.peer_id)));

                        self.state = State::Shutdown;
                    }
                    NodeMessage::Advertise(hash_set) => {
                        self.output
                            .push_back(OutMessage::WsMessage(WsMessage::Advertise(hash_set)));
                    }
                    NodeMessage::Withdraw(hash_set) => {
                        self.output
                            .push_back(OutMessage::WsMessage(WsMessage::Withdraw(hash_set)));
                    }
                }
            }
            InMessage::WsPeerClosed => {
                self.output
                    .push_back(OutMessage::BrokerMessage(BrokerMsg::UnRegisterPeer(
                        self.peer_id,
                    )));
                self.output
                    .push_back(OutMessage::WsCommand(WsCommand::PeerClosed(self.peer_id)));
                self.state = State::Shutdown;
            }
            InMessage::Shutdown => {
                self.output
                    .push_back(OutMessage::WsMessage(WsMessage::CloseConnection));
                self.output.push_back(OutMessage::ClosePeer);
                self.state = State::Shutdown;
            }
            InMessage::Unknown => {
                debug!("Got an unknown packet from the remote Websocket")
            } //TODO Needs to  be smarter
        }
    }
}

pub(crate) async fn run_ws_peer(
    mut stream: WebSockStream,
    // addr: SocketAddr,
    mut bus_control: watch::Receiver<BusControlMsg>,
    tx_command: UnboundedSender<WsCommand>,
    mut rx_control: UnboundedReceiver<WsControl>,
    mut peer: Peer,
) {
    trace!("Entered run_ws_peer");
    let mut ws_peer = WsPeer::new(peer.our_id, peer.peer_id);

    'outer: loop {
        // Send all queued outgoing messages first
        let mut in_message: Option<InMessage> = None;
        while let Some(out_message) = ws_peer.poll_outgoing() {
            trace!("WsPeer sending out_message: {:?}", &out_message);
            match out_message {
                OutMessage::WsCommand(cmd) => {
                    if let Err(e) = tx_command.send(cmd) {
                        eprintln!("Failed to send WsCommand: {}", e);
                        break 'outer; // everything is broken, exit loop
                    }
                }
                OutMessage::WsMessage(msg) => {
                    // let msg = Message::Binary(msg.into());
                    if let Err(e) = stream.send_msg(msg).await {
                        error!("Failed to send WsMessage: {} to {}", e, peer.peer_id);
                        in_message = Some(InMessage::WsPeerClosed);
                        break;
                    }
                }
                OutMessage::BrokerMessage(msg) => {
                    if peer.handle.send_broker_msg(msg).is_none() {
                        error!("Failed to send BrokerMsg to handle");
                        break 'outer; // everything is broken, exit loop
                    }

                    // Handle Broker message sending here
                }
                OutMessage::ClosePeer => {
                    stream.close_conn().await.ok();
                }
                OutMessage::ForwardPacket(wire_packet) => {
                    peer.handle.send_packet(wire_packet, peer.peer_id);
                }
            };
        }
        if let State::Shutdown = ws_peer.state {
            break;
        }

        in_message = if in_message.is_none() {
            select! {

                msg = stream.next_msg() => { Some(msg) },
                //     match msg {
                //         Some(Ok(message)) => {
                //             match message {
                //                 Message::Binary(data) => {
                //                     let msg = serde_cbor::from_slice::<WsMessage>(&data)
                //                         .map(|msg| InMessage::WsMessage(msg))
                //                         .unwrap_or_else(|e| {
                //                             error!("Failed to deserialize WsMessage: {}", e);
                //                             InMessage::WsPeerClosed
                //                         });
                //                     Some(msg)

                //                     // Some(InMessage::WsMessage(data))
                //                 }
                //                 Message::Close(_) => {
                //                     Some(InMessage::WsPeerClosed)
                //                 }
                //                 _ => None, // Ignore other message types for now
                //             }
                //         }
                //         Some(Err(e)) => {
                //             error!("WebSocket error: {}", e);
                //             Some(InMessage::WsPeerClosed)
                //         }
                //         None => {
                //             // Connection closed
                //             Some(InMessage::WsPeerClosed)
                //         }
                //     }
                // }
                Some(msg) = rx_control.recv() => {
                    Some(InMessage::WsControl(msg))
                }
                Some(msg) = peer.rx.recv() => {
                    Some(InMessage::NodeMessage(msg))
                }

                Ok(_msg) = bus_control.changed() => {
                    let msg = *bus_control.borrow();
                    match msg {
                        BusControlMsg::Shutdown => {
                            // ws_peer.state = State::Shutdown;
                            Some(InMessage::Shutdown)
                        }
                        _ => None,
                    }
                }
            }
        } else {
            in_message
        };

        if let Some(in_msg) = in_message {
            ws_peer.handle_input(in_msg);
        }
    }
}
