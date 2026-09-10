use std::collections::{HashMap, HashSet};

use crate::{
    EndpointId,
    errors::SendError,
    messages::{ClientMessage, NodeMessage},
    routing::{
        ConnectionId, Link, LsDb, NodeId, Packet, Payload, RouteKind, WirePacket,
        linkstate::{FibForwardTo, LsForwardTo},
    },
};

#[derive(Debug, Clone)]
pub(crate) struct ForwardingTable {
    pub(crate) our_id: NodeId,
    table: HashMap<EndpointId, FibEntry>,
    links: HashMap<ConnectionId, Link>,
    next_hop: HashMap<NodeId, Link>, // dest → neighbor
                                     // parent: HashMap<NodeId, NodeId>,
}

impl ForwardingTable {
    pub(crate) fn new(our_id: NodeId) -> ForwardingTable {
        ForwardingTable {
            our_id,
            table: HashMap::new(),
            links: HashMap::new(),
            next_hop: HashMap::new(),
            // parent: HashMap::new(),
        }
    }

    /// From local clients
    pub(crate) fn send(&self, packet: impl Into<Packet>) -> Result<(), SendError> {
        let packet = packet.into();
        tracing::debug!("Sending packet to {:?}", packet.to);
        tracing::debug!("FIB entry: {:?}", self.table.get(&packet.to.into()));
        let endpoint_id = packet.to.into();
        let Some(fib_entry) = self.table.get(&endpoint_id) else {
            return Err(SendError::NoRoute(packet.payload));
        };
        match fib_entry {
            FibEntry::Single(fib_forward) => match fib_forward {
                FibForwardTo::Local(sender) => {
                    sender
                        .try_send(ClientMessage::Message(packet.into()))
                        .map_err(|e| {
                            let ClientMessage::Message(p) = e.into_inner() else {
                                unreachable!()
                            };
                            SendError::NoRoute(p.payload)
                        })?;
                }
                FibForwardTo::Remote(next_hop) => {
                    next_hop
                        .tx
                        .try_send(crate::messages::NodeMessage::WirePacket(packet.into()))
                        .map_err(|e| {
                            let NodeMessage::WirePacket(wp) = e.into_inner().into() else {
                                unreachable!()
                            };
                            let payload = Payload::from(wp.payload);
                            SendError::NoRoute(payload)
                        })?;
                }
            },
            FibEntry::Multi(fib_forward_tos) => {
                for fib_forward in fib_forward_tos {
                    match fib_forward {
                        FibForwardTo::Local(sender) => {
                            let packet = packet.clone();
                            sender.try_send(ClientMessage::Message(packet.into())).ok();
                            // .map_err(|e| SendError::SendFailed(e.to_string()))?;
                        }
                        FibForwardTo::Remote(next_hop) => {
                            next_hop
                                .tx
                                .try_send(crate::messages::NodeMessage::WirePacket(
                                    packet.clone().into(),
                                ))
                                .ok();
                            // .map_err(|e| SendError::SendFailed(e.to_string()))?;
                        }
                    }
                }
            }
        };
        Ok(())
    }
    /// From remote peers
    #[cfg(feature = "remote")]
    pub(crate) fn forward(&self, packet: WirePacket, connection_id: ConnectionId) {
        tracing::debug!("Forwarding: {:?}", &packet);
        if packet.from == self.our_id {
            return; // shouldn't be forwarding our own packets
        }

        let from_hop = self.next_hop.get(&packet.from);
        if let Some(link) = from_hop {
            if link.connection_id != connection_id {
                tracing::debug!("Dropping packet RPF mismatch");
                return; // only forward packets that came from the right path
            }
        } else {
            return; // no route back just drop
        }

        let endpoint_id = packet.to.into();
        let Some(fib_entry) = self.table.get(&endpoint_id) else {
            return;
        };
        match fib_entry {
            FibEntry::Single(fib_forward) => match fib_forward {
                FibForwardTo::Local(sender) => {
                    sender
                        .try_send(ClientMessage::Message(packet.into()))
                        .unwrap_or_else(|e| {
                            tracing::error!("Failed to forward packet to local client: {}", e);
                        });
                }
                FibForwardTo::Remote(next_hop) => {
                    next_hop
                        .tx
                        .try_send(crate::messages::NodeMessage::WirePacket(packet))
                        .unwrap_or_else(|e| {
                            tracing::error!(
                                "Failed to forward packet on connection #{}: {}",
                                next_hop.connection_id,
                                e
                            );
                        });
                }
            },
            FibEntry::Multi(fib_forward_tos) => {
                'multi: for fib_forward in fib_forward_tos {
                    match fib_forward {
                        FibForwardTo::Local(sender) => {
                            let packet = packet.clone();
                            sender
                                .try_send(ClientMessage::Message(packet.into()))
                                .unwrap_or_else(|e| {
                                    tracing::error!(
                                        "Failed to forward packet to local client: {}",
                                        e
                                    );
                                });
                        }
                        FibForwardTo::Remote(link) => {
                            // check if the destination is back towards the sending node
                            let from = packet.from;
                            if let Some(from_hop) = self.next_hop.get(&from) {
                                if from_hop.peer_id == link.peer_id {
                                    tracing::debug!(
                                        "Not forwarding to {} due to back path",
                                        link.connection_id
                                    );
                                    continue 'multi;
                                }
                            }
                            link.tx
                                .try_send(crate::messages::NodeMessage::WirePacket(packet.clone()))
                                .unwrap_or_else(|e| {
                                    tracing::error!(
                                        "Failed to forward packet on connection #{}: {}",
                                        link.connection_id,
                                        e
                                    );
                                });
                        }
                    }
                }
            }
        };
    }

    pub(crate) fn get_node_id(&self) -> NodeId {
        self.our_id
    }
    // fn lookup(&self, address: &Address) -> Option<&ForwardTo> {
    //     None
    // }

    pub(crate) fn build_from_db(lsdb: &LsDb) -> ForwardingTable {
        let mut fib = ForwardingTable::new(lsdb.self_id);
        fib.links = lsdb.links.clone();
        fib.next_hop = lsdb.next_hop.clone();
        // fib.parent = lsdb.parent.clone();
        for (endpoint_id, route_entry) in lsdb.routes.routes().iter() {
            let fib_entry = match route_entry.kind {
                RouteKind::Unicast | RouteKind::Anycast | RouteKind::Node => {
                    if route_entry.routes.is_empty() {
                        continue;
                    }
                    let route = route_entry.routes.iter().min_by_key(|r| r.cost).unwrap();
                    let fib_forward = match &route.via {
                        LsForwardTo::Local(sender) => FibForwardTo::Local(sender.clone()),
                        LsForwardTo::Remote(node_id) => {
                            if let Some(fft) = lsdb
                                .next_hop
                                .get(node_id)
                                .map(|next_hop| FibForwardTo::Remote(next_hop.clone()))
                            {
                                fft
                            } else {
                                continue;
                            }
                        }
                    };
                    FibEntry::Single(fib_forward)
                }

                RouteKind::Broadcast => {
                    let mut forwards = Vec::new();
                    let mut remotes = HashSet::new();
                    for route in &route_entry.routes {
                        match &route.via {
                            LsForwardTo::Local(sender) => {
                                forwards.push(FibForwardTo::Local(sender.clone()));
                            }
                            LsForwardTo::Remote(node_id) => {
                                remotes.insert(node_id);
                            }
                        };
                    }

                    for node_id in remotes {
                        if let Some(next_hop) = lsdb.next_hop.get(&node_id) {
                            forwards.push(FibForwardTo::Remote(next_hop.clone()));
                        } else {
                            tracing::debug!("No next hop for remote node {}", node_id);
                        }
                    }

                    if forwards.is_empty() {
                        continue;
                    }

                    FibEntry::Multi(forwards)
                }
                RouteKind::Multicast => todo!(),
            };
            fib.table.insert(*endpoint_id, fib_entry);
        }
        fib
    }
}

#[derive(Debug, Clone)]
enum FibEntry {
    Single(FibForwardTo),
    Multi(Vec<FibForwardTo>),
}
