pub(crate) mod router;
pub(crate) mod routing_table;

use std::any::Any;

use erased_serde::Serializer;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{mpsc::UnboundedSender, oneshot, watch::error::SendError};
use uuid::Uuid;

use crate::{
    BusRider,
    messages::{ClientMessage, NodeMessage},
    routing::routing_table::RoutingTable,
};

pub(crate) type EndpointId = Uuid;
pub(crate) type NodeId = Uuid;
pub(crate) type PeerGroupId = Uuid;

#[derive(Debug, Clone, Default)]
pub(crate) struct ForwardingTable {
    table: std::collections::HashMap<EndpointId, ForwardTo>,
}

impl ForwardingTable {
    pub(crate) fn lookup(&self, endpoint_id: &EndpointId) -> Option<&ForwardTo> {
        self.table.get(endpoint_id)
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
        Self { table }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ForwardTo {
    Local(UnboundedSender<ClientMessage>),
    Remote(UnboundedSender<NodeMessage>),
    Broadcast(Vec<ForwardTo>), // List of Node IDs to broadcast to including myself
}

impl ForwardTo {
    pub(crate) fn send(&self, packet: impl Into<Packet>) -> Result<(), Payload> {
        let packet: Packet = packet.into();
        match self {
            ForwardTo::Local(tx) => tx.send(ClientMessage::Message(packet)).map_err(|e| {
                if let ClientMessage::Message(packet) = e.0 {
                    packet.payload
                } else {
                    unreachable!("Tried to send non-message to client")
                }
            }),
            ForwardTo::Remote(tx) => tx
                .send(NodeMessage::WirePacket(packet.into()))
                .map_err(|e| {
                    if let NodeMessage::WirePacket(packet) = e.0 {
                        packet.payload.into()
                    } else {
                        unreachable!("Tried to send non-message to client")
                    }
                }),
            ForwardTo::Broadcast(forward_tos) => {
                // let packet:Packet = packet;
                for ft in forward_tos {
                    ft.send(packet.clone())?;
                }
                Ok(())
            }
        }
    }
    // fn broadcast(
    //     &self,
    //     packet: impl Into<WirePacket>,
    //     destination_id: Uuid,
    // ) -> Result<(), Box<dyn std::error::Error>> {
    //     match self {
    //         ForwardTo::Local(tx) => tx
    //             .send(ClientMessage::Message(packet.into()))
    //             .map_err(|e| e.into()),
    //         ForwardTo::Remote(tx) => tx
    //             .send(NodeMessage::WirePacket(packet.into()))
    //             .map_err(|e| e.into()),
    //         ForwardTo::Broadcast(forward_tos) => {
    //             let packet = packet.into();
    //             for ft in forward_tos {
    //                 ft.broadcast(packet.clone(), destination_id)?;
    //             }
    //             Ok(())
    //         }
    //     }
    // }
}

#[derive(Debug, Clone)]
pub(crate) struct Packet {
    pub(crate) to: EndpointId,
    pub(crate) reply_to: Option<EndpointId>,
    pub(crate) from: Option<EndpointId>,
    pub(crate) payload: Payload,
}

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
    pub(crate) to: EndpointId,
    pub(crate) reply_to: Option<EndpointId>,
    pub(crate) from: Option<EndpointId>,
    pub(crate) payload: Vec<u8>,
}

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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum Address {
    Local(EndpointId),
    Remote(EndpointId, Uuid), // EndpointId, NodeId
}

impl From<EndpointId> for Address {
    fn from(value: EndpointId) -> Self {
        Address::Local(value)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Payload {
    BusRider(Box<dyn crate::traits::BusRider>),
    Bytes(Vec<u8>),
}

impl Payload {
    pub(crate) fn reveal<T: BusRider + for<'de> Deserialize<'de>>(self) -> Option<T> {
        match self {
            Payload::BusRider(br) => (br as Box<dyn Any>).downcast::<T>().ok().map(|b| *b),
            Payload::Bytes(bytes) => {
                let result = serde_cbor::from_slice(&bytes);
                result.ok()
            }
        }
    }
}

impl From<Payload> for Vec<u8> {
    fn from(value: Payload) -> Self {
        match value {
            Payload::BusRider(br) => {
                let mut v = Vec::new();
                let cbor = &mut serde_cbor::Serializer::new(serde_cbor::ser::IoWrite::new(&mut v));
                let mut cbor: Box<dyn Serializer> = Box::new(<dyn Serializer>::erase(cbor));
                br.erased_serialize(&mut cbor);
                drop(cbor);
                v
            }

            Payload::Bytes(b) => b,
        }
    }
}

impl From<Vec<u8>> for Payload {
    fn from(value: Vec<u8>) -> Self {
        Payload::Bytes(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)] //TODO
pub(crate) enum UnicastType {
    Datagram,
    Rpc,
    RpcResponse,
}

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
    pub(crate) realm: Realm,
    pub(crate) learned_from: PeerGroupId, // (0 for local)
    pub(crate) kind: RouteKind,
}

impl PartialEq for Route {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost
    }
}

impl PartialOrd for Route {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cost.cmp(&other.cost))
    }
}

impl Eq for Route {}

impl Ord for Route {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.cost.cmp(&other.cost)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub(crate) enum RouteKind {
    Unicast,
    Anycast,
    Multicast,
    Node,
}

/// Used to control how the route is advertised
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Realm {
    Process,
    Userspace,
    LocalNet,
    Global,
}

struct PeerGroup {
    id: PeerGroupId,
    peers: Vec<Uuid>,
}

#[derive(Debug, Error)]
pub(super) enum RouteTableError {
    #[error("Route kind didn't match")]
    DifferentRouteKind(RouteKind),
}
