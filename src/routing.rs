pub(crate) mod router;
pub(crate) mod routing_table;

#[cfg(feature = "serde")]
use erased_serde::Serializer;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use std::{
    any::Any,
    collections::{HashMap, HashSet},
    fmt::Display,
    ops::Deref,
};
use thiserror::Error;
use tokio::sync::mpsc::UnboundedSender;
use tracing::trace;
// use tracing::debug;
use uuid::Uuid;

#[cfg(feature = "remote")]
use crate::messages::NodeMessage;
use crate::{
    BusRider, errors::SendError, messages::ClientMessage, routing::routing_table::RoutingTable,
};

// pub(crate) type EndpointId = Uuid;
pub(crate) type NodeId = Uuid;

#[cfg(feature = "serde")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EndpointId(Uuid);

#[cfg(not(feature = "serde"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EndpointId(Uuid);

impl Display for EndpointId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Deref for EndpointId {
    type Target = Uuid;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<EndpointId> for Uuid {
    fn from(value: EndpointId) -> Self {
        value.0
    }
}

impl<'a> From<&'a EndpointId> for &'a Uuid {
    fn from(value: &'a EndpointId) -> Self {
        &value.0
    }
}
impl From<Uuid> for EndpointId {
    fn from(value: Uuid) -> Self {
        EndpointId(value)
    }
}

impl From<&Uuid> for EndpointId {
    fn from(value: &Uuid) -> Self {
        EndpointId(*value)
    }
}

#[cfg(feature = "remote")]
#[derive(Debug, Clone)]
pub(crate) struct PeerEntry {
    pub(crate) peer_tx: UnboundedSender<NodeMessage>,
    pub(crate) realm: Realm,
}

// impl From<PeerInfo> for PeerEntry {
//     fn from(value: PeerInfo) -> Self {
//         Self {
//             peer_tx: value.peer_tx,
//             realm: value.realm,
//         }
//     }
// }

#[derive(Debug, Clone, Default)]
pub(crate) struct ForwardingTable {
    table: std::collections::HashMap<EndpointId, ForwardTo>,
    node_id: NodeId,
    #[cfg(feature = "remote")]
    peers: HashMap<NodeId, PeerEntry>,
}

impl ForwardingTable {
    pub(crate) fn lookup(&self, address: &Address) -> Option<&ForwardTo> {
        match address {
            Address::Remote(eid, nid) => {
                trace!(
                    "Looking up remote address: {} on node {}",
                    nid, self.node_id
                );
                // If the node ID is my own, just look up the endpoint ID
                // Otherwise, look up the endpoint ID on the remote node
                // If the node ID is not in the table, return None
                let e = if nid == &self.node_id {
                    *eid
                } else {
                    let eid: EndpointId = nid.into();
                    eid
                };
                self.table.get(&e)
            }
            Address::Endpoint(eid) => self.table.get(eid),
        }
        // self.table.get(address)
    }

    pub(crate) fn get_node_id(&self) -> NodeId {
        self.node_id
    }

    #[cfg(feature = "remote")]
    pub(crate) fn forward(&self, packet: WirePacket, from_peer: NodeId) {
        let reverse_route = packet.from.map(|f| self.lookup(&f)).flatten();
        if reverse_route.is_none() {
            trace!("No reverse route for {:?}", packet);
            return;
        }
        trace!("Reverse Route {:?} for {:?}", reverse_route, packet);

        if let Some(ForwardTo::Remote(_, peer_id)) = reverse_route {
            if *peer_id != from_peer {
                trace!("Dropping due to RPF check");
                return;
            }
        }
        let endpoint_id = packet.to;
        _ = self.inner_send(endpoint_id, packet.into());
    }

    pub(crate) fn send(&self, packet: impl Into<Packet>) -> Result<(), SendError> {
        let mut packet: Packet = packet.into();
        let endpoint_id = packet.to;
        let node_id: EndpointId = self.node_id.into();
        packet.from = Some(node_id.into());

        self.inner_send(endpoint_id, packet).map_err(|p| {
            trace!("No route to endpoint_id: {}", endpoint_id);
            SendError::NoRoute(p.payload)
        })
    }

    fn inner_send(&self, endpoint_id: Address, packet: Packet) -> Result<(), Packet> {
        // let mut packet = packet.into();
        let forward_to = self.lookup(&endpoint_id);
        let forward_to = if let Some(ft) = forward_to {
            ft
        } else {
            return Err(packet);
        };
        // let packet: Packet = packet.into();
        match forward_to {
            ForwardTo::Local(tx) => tx.send(ClientMessage::Message(packet)).map_err(|e| {
                if let ClientMessage::Message(packet) = e.0 {
                    packet
                } else {
                    unreachable!("Tried to send non-message to client")
                }
            }),
            #[cfg(feature = "remote")]
            ForwardTo::Remote(tx, _node_id) => tx
                .send(NodeMessage::WirePacket(packet.into()))
                .map_err(|e| {
                    if let NodeMessage::WirePacket(packet) = e.0 {
                        packet.into()
                    } else {
                        unreachable!("Tried to send non-message to client")
                    }
                }),
            ForwardTo::Multicast(addresses) => {
                // let packet:Packet = packet;
                for address in addresses {
                    // TODO Currently ignoring errors when broadcasting
                    // Should we collect and return them all?
                    _ = self.inner_send(*address, packet.clone());

                    // ft.send(packet.clone())?;
                }
                Ok(())
            }
            ForwardTo::Broadcast(senders, realm) => {
                #[cfg(feature = "remote")]
                trace!(
                    "Broadcasting packet to {} clients and {} peers",
                    senders.len(),
                    self.peers.len()
                );
                #[cfg(not(feature = "remote"))]
                trace!("Broadcasting packet to {} clients", senders.len(),);
                for tx in senders {
                    _ = tx.send(ClientMessage::Message(packet.clone()));
                }
                #[cfg(feature = "remote")]
                for (nid, peer_entry) in self.peers.iter() {
                    if !realm.allows_realm(&peer_entry.realm) {
                        trace!(
                            "Not broadcasting to peer {} due to realm mismatch {:?} vs {:?}",
                            nid, realm, peer_entry.realm
                        );
                        continue;
                    }
                    trace!("Broadcasting to peer {}", nid);
                    _ = peer_entry
                        .peer_tx
                        .send(NodeMessage::WirePacket(packet.clone().into()));
                }

                Ok(())
            }
        }
    }
}

impl From<&RoutingTable> for ForwardingTable {
    fn from(value: &RoutingTable) -> Self {
        let mut table = std::collections::HashMap::new();
        for (endpoint_id, route_entry) in value.table.iter() {
            if let Some(best_route) = route_entry.best_route() {
                table.insert(*endpoint_id, best_route.via.clone());
            }
        }
        #[cfg(feature = "remote")]
        let peer_senders = value
            .peers
            .iter()
            .map(|(k, v)| (*k, v.peer_entry.clone()))
            .collect();

        #[cfg(feature = "remote")]
        let new = Self {
            peers: peer_senders,
            table,
            node_id: value.node_id,
        };

        #[cfg(not(feature = "remote"))]
        let new = Self {
            table,
            node_id: value.node_id,
        };

        new
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ForwardTo {
    Local(UnboundedSender<ClientMessage>),
    #[cfg(feature = "remote")]
    Remote(UnboundedSender<NodeMessage>, NodeId),
    Broadcast(Vec<UnboundedSender<ClientMessage>>, Realm),
    Multicast(HashSet<Address>), // List of Node IDs to broadcast to including myself
}

#[derive(Debug, Clone)]
pub(crate) struct Packet {
    pub(crate) to: Address,
    pub(crate) reply_to: Option<Address>,
    pub(crate) from: Option<Address>,
    pub(crate) payload: Payload,
}
#[cfg(feature = "remote")]
impl From<WirePacket> for Packet {
    fn from(value: WirePacket) -> Self {
        let payload = Payload::Bytes(value.payload);
        Self {
            to: value.to,
            reply_to: value.reply_to,
            from: value.from,
            payload,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct WirePacket {
    pub(crate) to: Address,
    pub(crate) reply_to: Option<Address>,
    pub(crate) from: Option<Address>,
    pub(crate) payload: Vec<u8>,
}

#[cfg(feature = "remote")]
impl From<Packet> for WirePacket {
    fn from(value: Packet) -> Self {
        let payload: Vec<u8> = value.payload.into();
        Self {
            to: value.to,
            reply_to: value.reply_to,
            from: value.from,
            payload,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, Hash, PartialEq, Eq)]
pub enum Address {
    Endpoint(EndpointId),
    Remote(EndpointId, NodeId), // EndpointId, NodeId
}

impl From<EndpointId> for Address {
    fn from(value: EndpointId) -> Self {
        Address::Endpoint(value)
    }
}

impl From<Uuid> for Address {
    fn from(value: Uuid) -> Self {
        Address::Endpoint(EndpointId(value))
    }
}

impl Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Address::Endpoint(eid) => write!(f, "Endpoint({})", eid),
            Address::Remote(eid, nid) => write!(f, "Remote(eid:{}, nid:{})", eid, nid),
        }
    }
}

#[cfg(not(feature = "remote"))]
#[derive(Debug, Clone)]
pub enum Payload {
    BusRider(Box<dyn crate::traits::BusRider>),
    // Bytes(Vec<u8>),
    // Packet(Box<Packet>), // For internal use only
}
#[cfg(not(feature = "remote"))]
impl Payload {
    pub(crate) fn reveal<T: BusRider>(self) -> Result<T, Self> {
        match self {
            Payload::BusRider(br) => {
                let res = (br as Box<dyn Any>).downcast::<T>().map(|b| *b);
                res.map_err(|e| e.into())
            }
        }
        // result.ok_or(self)
    }
}

#[cfg(feature = "remote")]
#[derive(Debug, Clone)]
pub enum Payload {
    BusRider(Box<dyn crate::traits::BusRider>),
    Bytes(Vec<u8>),
    // Packet(Box<Packet>), // For internal use only
}

#[cfg(feature = "remote")]
impl Payload {
    pub(crate) fn reveal<T: BusRider + for<'de> Deserialize<'de>>(self) -> Result<T, Self> {
        match self {
            Payload::BusRider(br) => {
                let res = (br as Box<dyn Any>).downcast::<T>().map(|b| *b);
                res.map_err(|e| e.into())
            }
            Payload::Bytes(bytes) => {
                let result = serde_cbor::from_slice(&bytes);
                result.map_err(|_| bytes.into())
            }
        }
        // result.ok_or(self)
    }
}

#[cfg(feature = "remote")]
impl From<Payload> for Vec<u8> {
    fn from(value: Payload) -> Self {
        match value {
            Payload::BusRider(br) => {
                let mut v = Vec::new();
                let cbor = &mut serde_cbor::Serializer::new(serde_cbor::ser::IoWrite::new(&mut v));
                let mut cbor: Box<dyn Serializer> = Box::new(<dyn Serializer>::erase(cbor));
                _ = br.erased_serialize(&mut cbor);
                drop(cbor);
                v
            }

            Payload::Bytes(b) => b,
        }
    }
}

#[cfg(feature = "remote")]
impl From<Vec<u8>> for Payload {
    fn from(value: Vec<u8>) -> Self {
        Payload::Bytes(value)
    }
}

impl From<Box<dyn Any>> for Payload {
    fn from(value: Box<dyn Any>) -> Self {
        match value.downcast::<Box<dyn crate::traits::BusRider>>() {
            Ok(b) => Payload::BusRider(*b),
            Err(_) => panic!("Tried to convert non-BusRider Box<dyn Any> into Payload"),
        }
    }
}

#[cfg(feature = "remote")]
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Advertisement {
    pub(crate) kind: RouteKind,
    pub(crate) cost: u16,
    pub(crate) endpoint_id: EndpointId,
}

#[derive(Debug, Clone)]
pub(crate) struct Route {
    pub(crate) via: ForwardTo,
    pub(crate) cost: u16,
    #[cfg(feature = "remote")]
    pub(crate) realm: Realm,
    #[cfg(feature = "remote")]
    pub(crate) learned_from: NodeId, // (0 for local)
    pub(crate) kind: RouteKind,
}

impl Route {
    pub(crate) fn add_broadcast(&mut self, other: Route) {
        if let ForwardTo::Multicast(ref mut list) = self.via {
            if let ForwardTo::Multicast(other_list) = other.via {
                other_list.into_iter().for_each(|a| {
                    list.insert(a);
                });
            }
        }
    }
}

impl PartialEq for Route {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost
    }
}

impl PartialOrd for Route {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        // Some(self.cost.cmp(&other.cost))
        Some(self.cmp(other))
    }
}

impl Eq for Route {}

impl Ord for Route {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.cost.cmp(&other.cost)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum RouteKind {
    Unicast,
    Anycast,
    Broadcast,
    Multicast,
    Node,
}

/// Used to control how the route is advertised
#[derive(Debug, Copy, Clone, PartialEq, Default, Eq, Serialize, Deserialize, Hash)]
#[allow(dead_code)]
pub enum Realm {
    /// Only within the current process
    Process,
    /// Within the current userspace instance (multiple processes on same machine)
    Userspace,
    /// Within the local network (LAN)
    LocalNet,
    /// Globally routable (Websocket)
    #[default]
    Global,
    // BroadcastProxy(EndpointId),
}

impl Realm {
    pub(crate) fn allows_realm(&self, other: &Realm) -> bool {
        match self {
            Realm::Global => true,
            Realm::LocalNet => matches!(other, Realm::LocalNet | Realm::Process | Realm::Userspace),
            Realm::Userspace => matches!(other, Realm::Userspace | Realm::Process),
            Realm::Process => matches!(other, Realm::Process),
        }
    }
}

#[derive(Debug, Error)]
pub(super) enum RouteTableError {
    #[error("Route kind didn't match")]
    DifferentRouteKind(RouteKind),
    #[error("Unicast route already exists")]
    UnicastRouteExists,
}
