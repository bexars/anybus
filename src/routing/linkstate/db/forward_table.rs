use std::collections::HashMap;
#[cfg(feature = "remote")]
use std::collections::HashSet;
#[cfg(feature = "remote")]
use std::iter::Peekable;

use crate::tokio;

#[cfg(feature = "remote")]
use tokio::sync::mpsc::Sender;

#[cfg(feature = "remote")]
use crate::routing::{Payload, WirePacket};
use crate::{
    EndpointId,
    errors::SendError,
    messages::ClientMessage,
    routing::{
        Cost, LsDb, NodeId, Packet, RouteKind,
        linkstate::{FibForwardTo, LsForwardTo, LsRoute},
    },
};
#[cfg(feature = "remote")]
use crate::{
    messages::NodeMessage,
    routing::{ConnectionId, Link},
};

#[derive(Debug, Clone)]
pub(crate) struct ForwardingTable {
    pub(crate) our_id: NodeId,
    table: HashMap<EndpointId, FibEntry>,
    #[cfg(feature = "remote")]
    links: HashMap<ConnectionId, Link>,
    #[cfg(feature = "remote")]
    next_hop: HashMap<NodeId, Link>, // dest → neighbor
                                     // parent: HashMap<NodeId, NodeId>,
}

impl ForwardingTable {
    pub(crate) fn new(our_id: NodeId) -> ForwardingTable {
        ForwardingTable {
            our_id,
            table: HashMap::new(),
            #[cfg(feature = "remote")]
            links: HashMap::new(),
            #[cfg(feature = "remote")]
            next_hop: HashMap::new(),
            // parent: HashMap::new(),
        }
    }

    /// From local clients
    pub(crate) fn send(&self, packet: Packet) -> Result<(), SendError> {
        tracing::debug!("Sending packet to {:?}", packet.to);
        tracing::trace!(
            "FIB entry: {:?}",
            self.table.get(&packet.to.get_endpoint(self.our_id))
        );
        let endpoint_id = packet.to.get_endpoint(self.our_id);
        let Some(fib_entry) = self.table.get(&endpoint_id) else {
            return Err(SendError::NoRoute(Some(packet.payload)));
        };
        match fib_entry {
            FibEntry::Single(fib_forward) => match fib_forward {
                FibForwardTo::Local(sender) => {
                    sender
                        .try_send(ClientMessage::Message(packet.into()))
                        .map_err(SendError::from)?;
                }
                #[cfg(feature = "remote")]
                FibForwardTo::Remote(next_hop) => {
                    next_hop
                        .tx
                        .try_send(NodeMessage::WirePacket(packet.into()))
                        .map_err(send_error_from_node)?;
                }
            },
            FibEntry::MultiCast(fib_forward_tos) => {
                for fib_forward in fib_forward_tos {
                    match fib_forward {
                        FibForwardTo::Local(sender) => {
                            let packet = packet.clone();
                            if let Err(err) = sender.try_send(ClientMessage::Message(packet.into()))
                            {
                                tracing::warn!(
                                    "Broadcast to local listener for {endpoint_id} failed: {err}"
                                );
                            }
                        }
                        #[cfg(feature = "remote")]
                        FibForwardTo::Remote(next_hop) => {
                            if let Err(err) = next_hop
                                .tx
                                .try_send(NodeMessage::WirePacket(packet.clone().into()))
                            {
                                tracing::warn!(
                                    "Broadcast to {} for {endpoint_id} failed: {err}",
                                    next_hop.peer_id
                                );
                            }
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

        let endpoint_id = packet.to.get_endpoint(self.our_id);
        let Some(fib_entry) = self.table.get(&endpoint_id) else {
            return;
        };
        let mut locals = fib_entry.locals().peekable();
        let mut remotes = fib_entry.remotes().peekable();
        let has_local = locals.peek().is_some();
        let has_remote = remotes.peek().is_some();
        let tx_local = |tx: &Sender<ClientMessage>, p| {
            tx.try_send(ClientMessage::Message(p)).unwrap_or_else(|e| {
                tracing::error!("Failed to forward packet to local client: {}", e);
            })
        };
        let tx_remote = |link: &Link, p: WirePacket| {
            let from = p.from;
            if let Some(from_hop) = self.next_hop.get(&from) {
                if from_hop.peer_id == link.peer_id {
                    tracing::trace!("Not forwarding to {} due to back path", link.connection_id);
                    return;
                }
            }
            link.tx
                .try_send(crate::messages::NodeMessage::WirePacket(p))
                .unwrap_or_else(|e| {
                    tracing::error!("Failed to forward packet to remote: {}", e);
                });
        };
        match (has_local, has_remote) {
            (true, true) => {
                Self::deliver(locals, packet.clone().into(), tx_local);
                Self::deliver(remotes, packet, tx_remote);
            }
            (true, false) => {
                Self::deliver(locals, packet.into(), tx_local);
            }
            (false, true) => {
                Self::deliver(remotes, packet, tx_remote);
            }
            (false, false) => {}
        }
    }

    #[cfg(feature = "remote")]
    fn deliver<I: std::fmt::Debug, T: std::fmt::Debug, F>(
        iter: Peekable<impl Iterator<Item = I>>,
        value: T,
        mut f: F,
    ) where
        T: Clone,
        F: FnMut(I, T),
    {
        let mut iter = iter.into_iter().peekable();

        while let Some(hop) = iter.next() {
            if iter.peek().is_some() {
                f(hop, value.clone());
            } else {
                f(hop, value);
                return;
            }
        }
    }

    pub(crate) fn get_node_id(&self) -> NodeId {
        self.our_id
    }
    // fn lookup(&self, address: &Address) -> Option<&ForwardTo> {
    //     None
    // }

    pub(crate) fn build_from_db(lsdb: &LsDb) -> ForwardingTable {
        let mut fib = ForwardingTable::new(lsdb.self_id);
        #[cfg(feature = "remote")]
        {
            fib.links = lsdb.links.clone();
            fib.next_hop = lsdb.next_hop.clone();
        }
        // fib.parent = lsdb.parent.clone();
        for (endpoint_id, route_entry) in lsdb.routes.routes().iter() {
            let fib_entry = match route_entry.kind {
                RouteKind::Anycast => {
                    let Some(route) = route_entry
                        .routes
                        .iter()
                        .filter_map(|route| anycast_total(lsdb, route).map(|total| (total, route)))
                        .min_by_key(|(total, _)| *total)
                        .map(|(_, route)| route)
                    else {
                        continue;
                    };
                    let Some(fib_forward) = fib_forward(lsdb, route) else {
                        continue;
                    };
                    FibEntry::Single(fib_forward)
                }
                RouteKind::Unicast | RouteKind::Node => {
                    if route_entry.routes.is_empty() {
                        continue;
                    }
                    let route = route_entry.routes.iter().min_by_key(|r| r.cost).unwrap();
                    let fib_forward = match &route.via {
                        LsForwardTo::Local(sender) => FibForwardTo::Local(sender.clone()),
                        #[cfg(feature = "remote")]
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
                    #[cfg(feature = "remote")]
                    let mut remotes = HashSet::new();
                    for route in &route_entry.routes {
                        match &route.via {
                            LsForwardTo::Local(sender) => {
                                forwards.push(FibForwardTo::Local(sender.clone()));
                            }
                            #[cfg(feature = "remote")]
                            LsForwardTo::Remote(node_id) => {
                                lsdb.next_hop.get(&node_id).map(|next_hop| {
                                    remotes.insert(next_hop.peer_id);
                                });
                            }
                        };
                    }
                    #[cfg(feature = "remote")]
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

                    FibEntry::MultiCast(forwards)
                }
                RouteKind::Multicast => todo!(),
            };
            fib.table.insert(*endpoint_id, fib_entry);
        }
        fib
    }
}

#[cfg_attr(not(feature = "remote"), allow(unused_variables))]
fn anycast_total(lsdb: &LsDb, route: &LsRoute) -> Option<Cost> {
    match &route.via {
        LsForwardTo::Local(_) => Some(route.cost),
        #[cfg(feature = "remote")]
        LsForwardTo::Remote(node_id) => {
            let path = lsdb.cost.get(node_id)?;
            if *path == Cost(u16::MAX) {
                return None;
            }
            lsdb.next_hop.get(node_id)?;
            Some(route.cost + *path)
        }
    }
}

#[cfg_attr(not(feature = "remote"), allow(unused_variables))]
fn fib_forward(lsdb: &LsDb, route: &LsRoute) -> Option<FibForwardTo> {
    match &route.via {
        LsForwardTo::Local(sender) => Some(FibForwardTo::Local(sender.clone())),
        #[cfg(feature = "remote")]
        LsForwardTo::Remote(node_id) => lsdb
            .next_hop
            .get(node_id)
            .map(|next_hop| FibForwardTo::Remote(next_hop.clone())),
    }
}

#[derive(Debug, Clone)]
enum FibEntry {
    Single(FibForwardTo),         // node, unicast and anycast only have one FibEntry
    MultiCast(Vec<FibForwardTo>), // eg. Broadcast before the renaming
}
impl FibEntry {
    #[cfg(feature = "remote")]
    fn hops(&self) -> std::slice::Iter<'_, FibForwardTo> {
        match self {
            FibEntry::Single(h) => std::slice::from_ref(h).iter(),
            FibEntry::MultiCast(v) => v.iter(),
        }
    }
    #[cfg(feature = "remote")]
    fn locals(&self) -> impl Iterator<Item = &Sender<ClientMessage>> {
        self.hops().filter_map(|h| match h {
            FibForwardTo::Local(tx) => Some(tx),
            _ => None,
        })
    }
    #[cfg(feature = "remote")]
    fn remotes(&self) -> impl Iterator<Item = &Link> {
        self.hops().filter_map(|h| match h {
            FibForwardTo::Remote(link) => Some(link),
            _ => None,
        })
    }
}

#[cfg(feature = "remote")]
fn send_error_from_node(
    err: crate::tokio::sync::mpsc::error::TrySendError<NodeMessage>,
) -> SendError {
    use crate::tokio::sync::mpsc::error::TrySendError;

    let (full, message) = match err {
        TrySendError::Full(message) => (true, message),
        TrySendError::Closed(message) => (false, message),
    };
    let payload = match message {
        NodeMessage::WirePacket(packet) => Some(Payload::from(packet.payload)),
        _ => None,
    };
    if full {
        SendError::Full(payload)
    } else {
        SendError::Closed(payload)
    }
}
