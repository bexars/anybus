use crate::tokio;

mod linkstate;
// pub(crate) mod peer_registry;
pub(crate) mod router;
// pub(crate) mod routing_table;
// use tokio_with_wasm::alias as tokio;

pub(crate) use linkstate::EndpointInfo;
pub(crate) use linkstate::ForwardingTable;
pub(crate) use linkstate::LsDb;
#[cfg(feature = "remote")]
pub(crate) use linkstate::LsaKey;
#[cfg(feature = "remote")]
pub(crate) use linkstate::{Link, Lsa};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "remote")]
use std::collections::HashSet;
#[cfg(feature = "remote")]
use std::sync::{
    Arc,
    atomic::{AtomicU16, Ordering},
};
use std::{
    any::Any,
    fmt::{Debug, Display},
    ops::Deref,
};
// use thiserror::Error;
use tokio::sync::mpsc::Sender;
// use tracing::debug;
use uuid::Uuid;

#[cfg(feature = "remote")]
use crate::messages::NodeMessage;
use crate::{BusRider, messages::ClientMessage};

// pub(crate) type EndpointId = Uuid;
// pub(crate) type NodeId = Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct NodeId(Uuid);

impl Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for NodeId {
    fn from(value: Uuid) -> Self {
        NodeId(value)
    }
}

impl From<&NodeId> for Uuid {
    fn from(value: &NodeId) -> Self {
        value.0
    }
}

impl From<NodeId> for Uuid {
    fn from(value: NodeId) -> Self {
        value.0
    }
}

impl NodeId {
    // pub fn nil() -> Self {
    //     NodeId(Uuid::nil())
    // }

    pub fn new() -> Self {
        NodeId(Uuid::now_v7())
    }
}

/// A newtype around Uuid
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
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

impl From<&NodeId> for EndpointId {
    fn from(value: &NodeId) -> Self {
        EndpointId(value.0)
    }
}

impl From<NodeId> for EndpointId {
    fn from(value: NodeId) -> Self {
        EndpointId(value.0)
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

impl EndpointId {
    /// Creates an EndpointId with a current timestamp v7 UUID under the hood
    pub fn new() -> Self {
        EndpointId(Uuid::now_v7())
    }
}

#[cfg(feature = "remote")]
#[derive(Debug, Clone)]
pub(crate) struct PeerEntry {
    pub(crate) peer_tx: Sender<NodeMessage>,
    // pub(crate) realm: Realm,
}

/// Used to control how the route is advertised
#[derive(Debug, Copy, Clone, PartialEq, Default, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[allow(dead_code)]
pub enum Realm {
    /// Only within the current process
    Process,
    /// Within the current userspace instance (multiple processes on same machine)
    #[cfg(feature = "remote")]
    Userspace,
    /// Within the local network (LAN)
    #[cfg(feature = "remote")]
    LocalNet,
    /// Globally routable (Websocket)
    #[default]
    Global,
    // BroadcastProxy(EndpointId),
}

#[derive(Clone)]
pub(crate) enum ForwardTo {
    Local(Sender<ClientMessage>),
    // Remote(Sender<NodeMessage>, ConnectionId),
    Broadcast(Vec<Sender<ClientMessage>>, Realm),
    // Multicast(HashSet<Address>), // List of Node IDs to broadcast to including myself
}

impl std::fmt::Debug for ForwardTo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local(_arg0) => f.debug_tuple("Local").finish(),
            // Self::Remote(_arg0, arg1) => f.debug_tuple("Remote").field(arg1).finish(),
            Self::Broadcast(arg0, arg1) => {
                write!(f, "Broadcast: {:?} {} entries", arg1, arg0.len())
            } // Self::Multicast(arg0) => write!(f, "Multicast: {} entries", arg0.len()),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Packet {
    pub(crate) to: Address,
    pub(crate) reply_to: Option<Address>,
    #[cfg_attr(not(feature = "remote"), allow(unused))]
    pub(crate) from: NodeId,
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
    pub(crate) from: NodeId,
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

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Address {
    Endpoint(EndpointId),
    Remote(EndpointId, NodeId), // EndpointId, NodeId
}

impl Address {
    pub(crate) fn get_endpoint(&self, local_node_id: NodeId) -> EndpointId {
        match self {
            Address::Endpoint(endpoint_id) => *endpoint_id,
            Address::Remote(endpoint_id, node_id) => {
                if local_node_id == *node_id {
                    *endpoint_id
                } else {
                    EndpointId::from(*node_id)
                }
            }
        }
    }
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
            Payload::Bytes(bytes) => crate::codec::decode(&bytes).map_err(|_| bytes.into()),
        }
        // result.ok_or(self)
    }
}

#[cfg(feature = "remote")]
impl From<Payload> for Vec<u8> {
    fn from(value: Payload) -> Self {
        match value {
            Payload::BusRider(br) => br.encode_payload(),

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
    pub(crate) cost: Cost,
    pub(crate) endpoint_id: EndpointId,
}

#[derive(Debug, Clone)]
pub(crate) struct Route {
    pub(crate) _via: ForwardTo,
    pub(crate) cost: Cost,
    pub(crate) realm: Realm,
    #[cfg(feature = "remote")]
    pub(crate) _learned_from: ConnectionId, // (0 for local)
    pub(crate) kind: RouteKind,
}

// impl Route {
//     pub(crate) fn add_broadcast(&mut self, other: Route) {
//         if let ForwardTo::Multicast(ref mut list) = self.via {
//             if let ForwardTo::Multicast(other_list) = other.via {
//                 other_list.into_iter().for_each(|a| {
//                     list.insert(a);
//                 });
//             }
//         }
//     }
// }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
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

#[cfg(feature = "remote")]
// impl Realm {
//     pub(crate) fn allow_broadcast(&self, other: &Realm) -> bool {
//         match self {
//             Realm::Global => true,
//             Realm::LocalNet => matches!(other, Realm::LocalNet | Realm::Userspace | Realm::Process),
//             Realm::Userspace => matches!(other, Realm::Userspace | Realm::Process),
//             Realm::Process => false, // Process realm cannot broadcast to other processes
//         }
//     }
// }
#[cfg(feature = "remote")]
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub(crate) struct RealmList(HashSet<Realm>);

#[cfg(feature = "remote")]
impl RealmList {
    // pub(crate) fn new(realm: Realm) -> Self {
    //     let mut rl = RealmList::default();
    //     rl.add(realm);
    //     rl
    // }

    #[allow(unused)]
    pub(crate) fn add(&mut self, realm: Realm) {
        self.0.insert(realm);
    }

    pub(crate) fn contains(&self, realm: &Realm) -> bool {
        self.0.contains(realm)
    }

    pub(crate) fn intersection(&self, other: &RealmList) -> RealmList {
        let intersection = self.0.intersection(&other.0).cloned().collect();
        RealmList(intersection)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
#[cfg(feature = "remote")]
impl From<Realm> for RealmList {
    fn from(realm: Realm) -> Self {
        let mut list = HashSet::new();
        list.insert(realm);
        RealmList(list)
    }
}

// #[derive(Debug, Error)]
// pub(super) enum RouteTableError {
//     #[error("Route kind didn't match")]
//     DifferentRouteKind(RouteKind),
//     #[error("Unicast route already exists")]
//     UnicastRouteExists,
// }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub(crate) struct Cost(u16);

impl std::ops::Add for Cost {
    type Output = Cost;

    fn add(self, rhs: Self) -> Self::Output {
        Cost(self.0.saturating_add(rhs.0))
    }
}

// impl std::ops::AddAssign for Cost {
//     fn add_assign(&mut self, rhs: Self) {
//         self.0 += rhs.0;
//     }
// }

impl std::ops::Add<u16> for Cost {
    type Output = Cost;

    fn add(self, rhs: u16) -> Self::Output {
        Cost(self.0.saturating_add(rhs))
    }
}

impl std::ops::AddAssign<u16> for Cost {
    fn add_assign(&mut self, rhs: u16) {
        self.0 += rhs;
    }
}

impl From<u16> for Cost {
    fn from(value: u16) -> Self {
        Cost(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg(feature = "remote")]

pub(crate) struct ConnectionId(u16);
#[cfg(feature = "remote")]

impl From<u16> for ConnectionId {
    fn from(value: u16) -> Self {
        ConnectionId(value)
    }
}
#[cfg(feature = "remote")]

impl Display for ConnectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone)]
#[cfg(feature = "remote")]

pub(crate) struct ConnectionIdCounter {
    // Arc allows multiple tasks to own a reference to this same memory
    current: Arc<AtomicU16>,
}
#[cfg(feature = "remote")]
impl std::fmt::Debug for ConnectionIdCounter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SharedCounter {{ current: {} }}",
            self.current.load(Ordering::Relaxed)
        )
    }
}
#[cfg(feature = "remote")]
impl ConnectionIdCounter {
    pub(crate) fn new() -> Self {
        ConnectionIdCounter {
            current: Arc::new(AtomicU16::new(1)),
        }
    }

    pub(crate) fn next(&self) -> ConnectionId {
        // Fetch the current value and increment it by 1 atomically
        self.current.fetch_add(1, Ordering::Relaxed).into()
    }
}
