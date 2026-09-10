//! Linkstate module handles all routing decisions for Anybus
//! LSAs received over the network are stored in the LsDb and are inserted into RouteTable with correct costs
//! Local Endpoint are inserted into the RouteTable and on the first entry or lower cost entry it will
//! create/update a LSA in the LsDb.  
//! The Forwarding table is created from the routing table by only putting best path in the table
//!
//! This separation allows multiple listeners locally to appear to be just one remotely.  Especially useful for
//! Anycast and Multicast routes since there's no need for multiple LSAs to be sent since there's no failover benefit
//! to knowing both

mod db;

pub(crate) use db::ForwardingTable;
pub(crate) use db::LsDb;
use std::fmt::Debug;
use tokio::time::Instant;
use tokio_with_wasm::alias as tokio;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;
use uuid::Uuid;

use crate::{
    Realm,
    messages::{ClientMessage, NodeMessage},
    routing::{ConnectionId, Cost, NodeId, RealmList, Route, RouteKind},
};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct Lsa {
    key: LsaKey, // (origin: NodeId, kind: Router | Endpoint, id)
    seq: u64,    // monotonic per key for this incarnation
    dead: bool,  // withdraw / max-age
    body: LsaBody,
}

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq, Hash, Clone, Copy)]
pub(crate) struct LsaKey {
    origin: NodeId,
    endpoint_id: Uuid, // Router: origin (or nil). Endpoint: EndpointId
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum LsaBody {
    Router(Vec<Adjacency>),
    Endpoint(EndpointInfo),
}

#[derive(Debug)]
pub(crate) struct LsRoute {
    pub(crate) via: LsForwardTo,
    pub(crate) cost: Cost,
    #[cfg(feature = "remote")]
    pub(crate) realm: Realm,
    #[cfg(feature = "remote")]
    pub(crate) kind: RouteKind,
}

#[derive(Clone)]
pub(crate) enum LsForwardTo {
    Local(Sender<ClientMessage>),
    Remote(NodeId), // Consult the LsDb for nexthops
}

impl std::fmt::Debug for LsForwardTo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LsForwardTo::Local(_sender) => write!(f, "Local(Sender<ClientMessage>)"),
            LsForwardTo::Remote(node_id) => write!(f, "Remote({}", node_id.0),
        }
    }
}

impl LsForwardTo {
    pub(crate) fn is_local(&self) -> bool {
        match self {
            LsForwardTo::Local(_sender) => true,
            LsForwardTo::Remote(_) => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EndpointInfo {
    pub(crate) kind: RouteKind,
    pub(crate) realm: Realm,
    pub(crate) cost: Cost,
}

impl From<&Route> for EndpointInfo {
    fn from(route: &Route) -> Self {
        EndpointInfo {
            kind: route.kind,
            realm: route.realm,
            cost: route.cost,
        }
    }
}

impl From<&LsRoute> for EndpointInfo {
    fn from(ls_route: &LsRoute) -> Self {
        EndpointInfo {
            kind: ls_route.kind,
            realm: ls_route.realm,
            cost: ls_route.cost,
        }
    }
}

#[derive(Debug)]
struct LsRouteEntry {
    routes: Vec<LsRoute>,
    kind: RouteKind,
}

impl LsRouteEntry {
    // fn min_cost(&self) -> Cost {
    //     self.routes
    //         .iter()
    //         .map(|r| r.cost)
    //         .min()
    //         .unwrap_or(Cost(u16::MAX)) // Should always be a route, but just in case
    // }
}

#[derive(Debug, Clone)]
enum FibForwardTo {
    Local(Sender<ClientMessage>),
    Remote(Link),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Adjacency {
    connection_id: ConnectionId,
    peer_id: NodeId,
    cost: Cost,
    realms: RealmList,
}

#[derive(Debug)]
struct LsaRecord {
    lsa: Lsa,
    updated_at: Instant,
}

#[derive(Debug, Clone)]
pub(crate) struct Link {
    tx: Sender<NodeMessage>,
    peer_id: NodeId,
    connection_id: ConnectionId,
    realms: RealmList,
    cost: Cost,
    _default_route: bool,
    _role_stub: bool,
}

impl Link {
    pub(crate) fn new(
        tx: Sender<NodeMessage>,
        peer_id: NodeId,
        connection_id: ConnectionId,
        realms: RealmList,
        cost: Cost,
        _default_route: bool,
        _role_stub: bool,
    ) -> Self {
        Link {
            tx,
            peer_id,
            connection_id,
            realms,
            cost,
            _default_route,
            _role_stub,
        }
    }

    pub(crate) fn send_lsa(&self, mut lsa: Lsa) -> Result<(), LinkError> {
        match lsa.body {
            LsaBody::Router(ref mut adjacencies) => {
                adjacencies.iter_mut().for_each(|adj| {
                    adj.realms = adj.realms.intersection(&self.realms);
                });
                adjacencies.retain(|adj| !adj.realms.is_empty());
                if adjacencies.is_empty() {
                    return Err(LinkError::RealmMismatch);
                }
            }
            LsaBody::Endpoint(ref endpoint_info) => {
                if !self.realms.contains(&endpoint_info.realm) {
                    return Err(LinkError::RealmMismatch);
                }
            }
        }
        self.tx.try_send(NodeMessage::Lsa(lsa))?;
        Ok(())
    }

    pub(crate) fn send_lsa_ack(&self, key: LsaKey, seq: u64) -> Result<(), LinkError> {
        self.tx.try_send(NodeMessage::LsaAck { key, seq })?;
        Ok(())
    }
}

#[allow(unused)]
#[derive(Debug)]
pub(crate) enum LinkError {
    TrySendError(tokio::sync::mpsc::error::TrySendError<NodeMessage>),
    RealmMismatch,
}

// impl PartialEq for LinkError {
//     fn eq(&self, other: &Self) -> bool {
//         self == other
//     }
// }

impl From<tokio::sync::mpsc::error::TrySendError<NodeMessage>> for LinkError {
    fn from(err: tokio::sync::mpsc::error::TrySendError<NodeMessage>) -> Self {
        LinkError::TrySendError(err)
    }
}

#[derive(Eq, Debug)]
struct Entry(Cost, NodeId);

impl Entry {
    pub(crate) fn better(&self, other: &Self) -> bool {
        self < other
    }
}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(other.cmp(&self))
    }
}

// We want the lowest cost and the lowest UUID to win
impl Ord for Entry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.0.cmp(&self.0).then(self.1.cmp(&other.1))
    }
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

#[cfg(test)]
mod test {
    use uuid::{Uuid, uuid};

    use crate::routing::linkstate::Entry;

    #[test]
    fn test_entry_ord() {
        let uuid1 = uuid!("d87e3825-9c8f-4a0b-85d1-ef8c5d28a3f1");
        let uuid2 = uuid!("d87e3825-9c8f-4a0b-85d1-ef8c5d28a3f2");
        let entry1 = Entry(1.into(), uuid1.into());
        let entry2 = Entry(1.into(), uuid2.into());
        let entry3 = Entry(2.into(), uuid1.into());
        assert!(entry1 == entry1);
        assert!(entry1 < entry3);
        assert!(entry1 > entry2);
    }

    #[test]
    fn test_uuid7_now() {
        let uuid1 = Uuid::now_v7();
        let uuid2 = Uuid::now_v7();
        let entry1 = Entry(1.into(), uuid1.into());
        let entry2 = Entry(1.into(), uuid2.into());

        assert!(entry1 > entry2);
    }
}
