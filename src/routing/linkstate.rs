use std::{
    collections::{HashMap, VecDeque},
    fmt::Debug,
    time::Instant,
};

use itertools::Itertools;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;
use uuid::Uuid;

use crate::{
    EndpointId, Realm,
    messages::NodeMessage,
    routing::{ConnectionId, Cost, NodeId, RouteKind},
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
    id: Uuid, // Router: origin (or nil). Endpoint: EndpointId
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum LsaBody {
    Router(Vec<Adjacency>),
    EndPoint(EndpointId, RouteKind, Realm),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Adjacency {
    connection_id: ConnectionId,
    peer_id: NodeId,
    cost: Cost,
    realm: Realm,
}

#[derive(Debug)]
struct LsaRecord {
    lsa: Lsa,
    updated_at: Instant,
}

#[derive(Debug)]
pub(crate) struct Link {
    tx: Sender<NodeMessage>,
    peer_id: NodeId,
    connection_id: ConnectionId,
    realm: Realm,
    cost: Cost,
    _default_route: bool,
    _role_stub: bool,
}

impl Link {
    pub(crate) fn new(
        tx: Sender<NodeMessage>,
        peer_id: NodeId,
        connection_id: ConnectionId,
        realm: Realm,
        cost: Cost,
        _default_route: bool,
        _role_stub: bool,
    ) -> Self {
        Link {
            tx,
            peer_id,
            connection_id,
            realm,
            cost,
            _default_route,
            _role_stub,
        }
    }
}

#[derive(Debug)]
pub(crate) struct LsDb {
    db: HashMap<LsaKey, LsaRecord>,
    self_id: NodeId,
    links: HashMap<ConnectionId, Link>,
    pending_tx: HashMap<(ConnectionId, LsaKey), u64>,
    next_hop: HashMap<NodeId, NodeId>, // dest → neighbor
    cost: HashMap<NodeId, Cost>,
    parent: HashMap<NodeId, NodeId>,
}

impl LsDb {
    pub(crate) fn new(self_id: NodeId) -> LsDb {
        LsDb {
            db: HashMap::new(),
            self_id,
            links: HashMap::new(),
            pending_tx: HashMap::new(),
            next_hop: HashMap::new(),
            cost: HashMap::new(),
            parent: HashMap::new(),
        }
    }

    /// Returns true if the forwarding table needs to be updated
    pub(crate) fn handle_lsa(&mut self, lsa: Lsa, in_connection_id: ConnectionId) -> bool {
        let mut dirty = false;
        let run_spf = if let LsaBody::Router(_) = lsa.body {
            true
        } else {
            false
        };

        let ack = NodeMessage::LsaAck {
            key: lsa.key,
            seq: lsa.seq,
        };
        // self.pending_tx.insert((in_connection_id, lsa.key), lsa.seq);
        let seq = lsa.seq;
        let key = lsa.key;
        if let Some(current_lsa) = self.db.get_mut(&key) {
            if current_lsa.lsa.seq < seq {
                current_lsa.lsa = lsa.clone();
                current_lsa.updated_at = Instant::now();
                dirty = true;
            } else {
                tracing::warn!(
                    "Received LSA with seq {} which is not newer than current seq {} for key {:?}",
                    lsa.seq,
                    current_lsa.lsa.seq,
                    lsa.key
                );
                return false;
            }
        } else {
            self.db.insert(
                lsa.key,
                LsaRecord {
                    lsa: lsa.clone(),
                    updated_at: Instant::now(),
                },
            );
            dirty = true;
        }

        let Some(in_link) = self.links.get(&in_connection_id) else {
            tracing::error!(
                "Received LSA from unknown connection_id {:?}. Cannot send ack.",
                in_connection_id
            );
            return dirty;
        };

        for (out_conn_id, out_link) in self.links.iter() {
            if out_conn_id == &in_connection_id {
                continue; // Don't send back to the sender
            }
            if lsa.key.origin == out_link.peer_id {
                continue;
            }; // don't forward back to the originator
            if self.allow_flood(&in_link.realm, &out_link.realm) {
                let lsa_message = NodeMessage::Lsa(lsa.clone());
                if let Err(e) = out_link.tx.try_send(lsa_message) {
                    tracing::error!(
                        "Failed to forward LSA to peer {:?} over connection {:?}: {:?}",
                        out_link.peer_id,
                        out_conn_id,
                        e
                    );
                }
                self.pending_tx.insert((out_link.connection_id, key), seq);
            }
        }

        in_link.tx.try_send(ack).unwrap_or_else(|e| {
            tracing::error!(
                "Failed to send LSA Ack to peer {:?} over connection {:?}: {:?}",
                in_link.peer_id,
                in_connection_id,
                e
            );
        });

        if dirty && run_spf {
            self.compute_spf();
        };
        self.compute_spf();
        dirty
    }

    pub(crate) fn handle_ack(&mut self, from: ConnectionId, key: LsaKey, seq: u64) {
        tracing::trace!("Got an ack from: {} key: {:?} seq: {}", &from, &key, &seq);
        if let Some(val) = self.pending_tx.remove(&(from, key)) {
            if val != seq {
                self.pending_tx.insert((from, key), val);
                tracing::warn!("Re-inserting a pending ack");
            }
            return;
        } else {
            tracing::warn!("Couldn't find pending ack to remove {} {:?}", from, key);
        };
        dbg!(&self.pending_tx);
    }

    pub(crate) fn add_peer(&mut self, link: Link) {
        let connection_id = link.connection_id;
        self.add_adjacency(link.peer_id, link.connection_id, link.realm, link.cost);
        self.links.insert(link.connection_id, link);
        self.flood_peer(connection_id);
        self.compute_spf();
        self.purge_unreachable();
    }

    fn add_adjacency(
        &mut self,
        peer_id: NodeId,
        connection_id: ConnectionId,
        realm: Realm,
        cost: Cost,
    ) {
        let adjacency = Adjacency {
            peer_id,
            cost, // TODO: determine cost based on link properties
            realm,
            connection_id,
        };

        let lsa_key = LsaKey {
            origin: self.self_id,
            id: self.self_id.0,
        };

        let root_lsa = self.db.entry(lsa_key.clone()).or_insert_with(|| {
            let lsa = Lsa {
                key: lsa_key,
                seq: 0,
                dead: false,
                body: LsaBody::Router(vec![]),
            };
            let record = LsaRecord {
                lsa,
                updated_at: Instant::now(),
            };
            record
        });

        if let LsaRecord {
            lsa:
                Lsa {
                    body: LsaBody::Router(adjacencies),
                    ..
                },
            ..
        } = root_lsa
        {
            adjacencies.push(adjacency);
        } else {
            tracing::error!("LSA body is not a Router type for node_id: {:?}", peer_id);
        }
    }

    pub(crate) fn remove_peer(&mut self, connection_id: ConnectionId) {
        tracing::info!("LsDb removing peer on connection {}", connection_id);
        if let Some(link) = self.links.remove(&connection_id) {
            let lsa_key = LsaKey {
                origin: self.self_id,
                id: self.self_id.0,
            };

            if let Some(record) = self.db.get_mut(&lsa_key) {
                if let LsaBody::Router(adjacencies) = &mut record.lsa.body {
                    let len = adjacencies.len();
                    adjacencies.retain(|adj| adj.connection_id != connection_id);
                    if adjacencies.len() < len {
                        tracing::info!(
                            "Removed peer with connection_id {:?} from LSA for node_id {:?}.",
                            connection_id,
                            link.peer_id
                        );
                        record.lsa.seq += 1;
                        record.updated_at = std::time::Instant::now();
                        for link in &self.links {
                            link.1
                                .tx
                                .try_send(NodeMessage::Lsa(record.lsa.clone()))
                                .ok();
                        }
                    } else {
                        tracing::warn!(
                            "Attempted to remove peer with connection_id {:?}, but it was not found in LSA for node_id {:?}.",
                            connection_id,
                            link.peer_id
                        );
                    }
                }
            }
        } else {
            tracing::warn!(
                "Attempted to remove peer with connection_id {:?}, but no such link exists.",
                connection_id
            );
            return;
        }

        self.compute_spf();
        self.purge_unreachable();
    }

    pub(crate) fn purge_unreachable(&mut self) {
        let list: Vec<_> = self
            .cost
            .extract_if(|k, v| *v == u16::MAX.into())
            .map(|(k, _v)| k)
            .collect();

        for id in list {
            self.db.retain(|k, _| k.origin != id);
        }
    }

    pub(crate) fn flood_peer(&mut self, connection_id: ConnectionId) {
        if let Some(link) = self.links.get(&connection_id) {
            for record in self.db.values() {
                if record.lsa.key.origin == link.peer_id || record.lsa.dead {
                    continue; // Don't send the peer its own LSA or dead LSAs
                }
                let lsa_message = NodeMessage::Lsa(record.lsa.clone());
                self.pending_tx
                    .insert((link.connection_id, record.lsa.key.clone()), record.lsa.seq);
                if let Err(e) = link.tx.try_send(lsa_message) {
                    tracing::error!(
                        "Failed to send LSA to peer {:?} over connection {:?}: {:?}",
                        link.peer_id,
                        connection_id,
                        e
                    );
                }
            }
        } else {
            tracing::warn!(
                "Attempted to flood peer with connection_id {:?}, but no such link exists.",
                connection_id
            );
        }
    }

    fn compute_spf(&mut self) {
        self.next_hop.clear();
        self.cost.clear();
        self.parent.clear();

        let nodes = self
            .db
            .iter()
            .filter_map(|(k, v)| {
                if let LsaBody::Router(router) = &v.lsa.body {
                    Some((k.origin, router))
                } else {
                    None
                }
            })
            .collect::<HashMap<_, _>>();

        let root = self.self_id;
        // dbg!(&nodes);
        for node_id in nodes.keys() {
            self.cost.insert(*node_id, u16::MAX.into());
            // self.parent.insert(*node_id, Uuid::nil().into());
        }
        self.cost.insert(root, Cost(0));

        let mut queue = VecDeque::new();
        queue.push_back(Entry(Cost(0), root));

        while let Some(Entry(current_cost, current_node)) = queue.pop_front() {
            // dbg!(current_cost, &current_node, &queue);
            if let Some(adjacencies) = nodes.get(&current_node) {
                for adjacency in adjacencies.into_iter() {
                    let next_node = adjacency.peer_id;
                    let new_cost = current_cost + adjacency.cost;

                    if new_cost < *self.cost.get(&next_node).unwrap_or(&Cost(u16::MAX)) {
                        self.cost.insert(next_node, new_cost);
                        self.parent.insert(next_node, current_node);
                        // self.next_hop.insert(current_node, adjacency.node_id);
                        let entry = Entry(new_cost, next_node);
                        if let Some((pos, _)) =
                            queue.iter().find_position(|Entry(_, n)| n == &next_node)
                        {
                            queue.remove(pos);
                        };
                        let pos = queue.partition_point(|e| e.0 < entry.0);
                        queue.insert(pos, entry);
                    }
                }
            }
        }
    }

    fn allow_flood(&self, from: &Realm, to: &Realm) -> bool {
        if to == from {
            return true;
        };
        if matches!(from, Realm::Process) {
            return false;
        };
        false
    }
}

#[derive(Eq, Debug)]
struct Entry(Cost, NodeId);
impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(other.0.cmp(&self.0))
    }
}
impl Ord for Entry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.0.cmp(&self.0)
    }
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
