pub(crate) mod router;
pub(crate) mod routing_table;
use tokio_with_wasm::alias as tokio;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "remote")]
use std::collections::HashMap;
use std::{
    any::Any,
    collections::HashSet,
    fmt::{Debug, Display},
    ops::Deref,
};
use thiserror::Error;
use tokio::sync::mpsc::{Sender, error::TrySendError};
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
    pub(crate) peer_tx: Sender<NodeMessage>,
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

#[derive(Clone, Default)]
pub(crate) struct ForwardingTable {
    table: std::collections::HashMap<EndpointId, ForwardTo>,
    node_id: NodeId,
    #[cfg(feature = "remote")]
    peers: HashMap<NodeId, PeerEntry>,
}

impl Debug for ForwardingTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Node: {}", self.node_id)?;
        self.table
            .iter()
            .try_for_each(|(k, v)| -> std::fmt::Result {
                write!(f, "{}", k)?;
                write!(f, "{:?}", v)
            })?;
        self.peers
            .iter()
            .try_for_each(|(k, v)| -> std::fmt::Result {
                write!(f, "Node: {} {:?}", k, v.realm)
            })?;

        f.debug_struct("ForwardingTable")
            .field("table", &self.table)
            .field("node_id", &self.node_id)
            .field("peers", &self.peers)
            .finish()
    }
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
        self.inner_send(endpoint_id, packet.into()).ok();
    }

    pub(crate) fn send(&self, packet: impl Into<Packet>) -> Result<(), SendError> {
        let mut packet: Packet = packet.into();
        let endpoint_id = packet.to;
        let node_id: EndpointId = self.node_id.into();
        packet.from = Some(node_id.into());

        self.inner_send(endpoint_id, packet)
        //         .map_err(|p| {
        //         trace!("No route to endpoint_id: {}", endpoint_id);
        //         SendError::NoRoute(p.payload)
        //     })
    }

    fn inner_send(&self, endpoint_id: Address, packet: Packet) -> Result<(), SendError> {
        // let mut packet = packet.into();
        let forward_to = self.lookup(&endpoint_id);
        let forward_to = if let Some(ft) = forward_to {
            ft
        } else {
            return Err(SendError::NoRoute(packet.payload));
        };
        // let packet: Packet = packet.into();
        let get_packet_payload = |cm: ClientMessage| {
            if let ClientMessage::Message(packet) = cm {
                packet.payload
            } else {
                unreachable!("Tried to send non-message to client")
            }
        };

        let get_nodemessage_payload = |cm: NodeMessage| -> Payload {
            if let NodeMessage::WirePacket(packet) = cm {
                let p: Packet = packet.into();
                p.payload
            } else {
                unreachable!("Tried to send non-message to client")
            }
        };
        match forward_to {
            ForwardTo::Local(tx) => {
                tx.try_send(ClientMessage::Message(packet))
                    .map_err(|e| match e {
                        TrySendError::Full(cm) => SendError::Full(get_packet_payload(cm)),
                        TrySendError::Closed(cm) => SendError::NoRoute(get_packet_payload(cm)), // need to flag a dead sender
                    })
            }
            #[cfg(feature = "remote")]
            ForwardTo::Remote(tx, _node_id) => tx
                .try_send(NodeMessage::WirePacket(packet.into()))
                .map_err(|e| match e {
                    TrySendError::Full(nm) => SendError::Full(get_nodemessage_payload(nm)),
                    TrySendError::Closed(nm) => SendError::NoRoute(get_nodemessage_payload(nm)), // need to flag a dead sender
                }),
            // .map_err(|e| {
            //     if let NodeMessage::WirePacket(packet) = e.0 {
            //         packet.into()
            //     } else {
            //         unreachable!("Tried to send non-message to client")
            //     }
            // }),
            ForwardTo::Multicast(addresses) => {
                // let packet:Packet = packet;
                for address in addresses {
                    // TODO Currently ignoring errors when broadcasting
                    // Should we collect and return them all?
                    self.inner_send(*address, packet.clone()).ok();

                    // ft.send(packet.clone())?;
                }
                Ok(())
            }
            #[allow(unused_variables)]
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
                    tx.try_send(ClientMessage::Message(packet.clone())).ok();
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
                    peer_entry
                        .peer_tx
                        .try_send(NodeMessage::WirePacket(packet.clone().into()))
                        .ok();
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

#[derive(Clone)]
pub(crate) enum ForwardTo {
    Local(Sender<ClientMessage>),
    #[cfg(feature = "remote")]
    Remote(Sender<NodeMessage>, NodeId),
    Broadcast(Vec<Sender<ClientMessage>>, Realm),
    Multicast(HashSet<Address>), // List of Node IDs to broadcast to including myself
}

impl std::fmt::Debug for ForwardTo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local(_arg0) => f.debug_tuple("Local").finish(),
            Self::Remote(_arg0, arg1) => f.debug_tuple("Remote").field(arg1).finish(),
            Self::Broadcast(arg0, arg1) => {
                write!(f, "Broadcast: {:?} {} entries", arg1, arg0.len())
            }
            Self::Multicast(arg0) => write!(f, "Multicast: {} entries", arg0.len()),
        }
    }
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

#[cfg(feature = "remote")]
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
                use erased_serde::Serializer;

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

impl Display for RouteKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            RouteKind::Unicast => "Unicast",
            RouteKind::Anycast => "Anycast",
            RouteKind::Broadcast => "Broadcast",
            RouteKind::Multicast => "Multicast",
            RouteKind::Node => "Node",
        };
        f.pad(s)
    }
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

#[cfg(feature = "remote")]
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
