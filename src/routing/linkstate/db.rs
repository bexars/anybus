use itertools::Itertools;

use std::{
    collections::{HashMap, VecDeque},
    fmt::Debug,
};

use web_time::Instant;

use crate::{
    EndpointId, Realm,
    messages::ClientMessage,
    routing::{
        ConnectionId, Cost, ForwardTo, Link, Lsa, LsaKey, NodeId, RealmList, Route, RouteKind,
        linkstate::{Adjacency, EndpointInfo, Entry, LsaBody, LsaRecord, route_table::RouteTable},
    },
};

use super::route_table::Effects;

#[derive(Debug)]
pub(crate) struct LsDb {
    db: HashMap<LsaKey, LsaRecord>,
    self_id: NodeId,
    links: HashMap<ConnectionId, Link>,
    pending_tx: HashMap<(ConnectionId, LsaKey), u64>,
    next_hop: HashMap<NodeId, NodeId>, // dest → neighbor
    cost: HashMap<NodeId, Cost>,
    parent: HashMap<NodeId, NodeId>, // need spf to populate this
    routes: RouteTable,
    rebuild_requested: Option<Instant>,
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
            routes: RouteTable::new(),
            rebuild_requested: None,
        }
    }

    fn remove_dead_lsa(&mut self, lsa: Lsa) {
        assert!(lsa.dead, "removed_dead_lsa called with non-dead LSA");
        if let Some(current_lsa) = self.db.get(&lsa.key) {
            if current_lsa.lsa.seq < lsa.seq {
                self.db.remove(&lsa.key);
                self.flood_all_neighbors(lsa.clone(), 0.into());

                self.request_rebuild();
            } else {
                tracing::warn!(
                    "Received dead LSA with seq {} which is not newer than current seq {} for key {:?}",
                    lsa.seq,
                    current_lsa.lsa.seq,
                    lsa.key
                );
                return;
            }
        }
        self.routes
            .remove_remote_endpoint(lsa.key.endpoint_id.into(), lsa.key.origin);
    }

    fn request_rebuild(&mut self) {
        if self.rebuild_requested.is_none() {
            self.rebuild_requested = Some(Instant::now());
        }
    }

    fn upsert_lsa(&mut self, lsa: &Lsa) -> Result<(), ()> {
        if let Some(current_lsa) = self.db.get_mut(&lsa.key) {
            if current_lsa.lsa.seq < lsa.seq {
                current_lsa.lsa = lsa.clone();
                current_lsa.updated_at = Instant::now();
                self.request_rebuild();
            } else {
                tracing::warn!(
                    "Received LSA with seq {} which is not newer than current seq {} for key {:?}",
                    lsa.seq,
                    current_lsa.lsa.seq,
                    lsa.key
                );
                return Err(());
            }
        } else {
            self.db.insert(
                lsa.key,
                LsaRecord {
                    lsa: lsa.clone(),
                    updated_at: Instant::now(),
                },
            );

            self.request_rebuild();
        }

        match lsa.body {
            LsaBody::Router(_) => {
                self.routes
                    .add_remote_endpoint(
                        lsa.key.endpoint_id.into(),
                        lsa.key.origin,
                        EndpointInfo {
                            kind: RouteKind::Node,
                            realm: Realm::Global,
                            cost: self
                                .cost
                                .get(&lsa.key.origin)
                                .cloned()
                                .unwrap_or(Cost(u16::MAX)),
                        },
                    )
                    .ok();
            }
            LsaBody::Endpoint(ref endpoint_info) => {
                self.routes
                    .add_remote_endpoint(
                        lsa.key.endpoint_id.into(),
                        lsa.key.origin,
                        endpoint_info.clone(),
                    )
                    .ok();
            }
        }

        Ok(())
    }

    fn send_ack(&self, in_connection_id: ConnectionId, key: LsaKey, seq: u64) {
        let Some(in_link) = self.links.get(&in_connection_id) else {
            tracing::error!(
                "Received LSA from unknown connection_id {:?}. Cannot send ack.",
                in_connection_id
            );
            return;
        };

        in_link.send_lsa_ack(key, seq).unwrap_or_else(|e| {
            tracing::error!(
                "Failed to send LSA Ack to peer {:?} over connection {:?}: {:?}",
                in_link.peer_id,
                in_connection_id,
                e
            );
        });
    }

    pub(crate) fn handle_lsa(&mut self, lsa: Lsa, in_connection_id: ConnectionId) {
        self.send_ack(in_connection_id, lsa.key, lsa.seq); // Regardless we let them know we got it

        if lsa.key.origin == self.self_id {
            tracing::warn!(
                "Ignoring LSA from self with seq {} for key {:?}",
                lsa.seq,
                lsa.key
            );
            return;
        }

        // handle dead lsa
        if lsa.dead {
            self.remove_dead_lsa(lsa);
            return;
        }

        let run_spf = if let LsaBody::Router(_) = lsa.body {
            true
        } else {
            false
        };

        if self.upsert_lsa(&lsa).is_err() {
            tracing::warn!(
                "Ignoring LSA with seq {} which is not newer than current seq for key {:?}",
                lsa.seq,
                lsa.key
            );
            return;
        };

        //
        let in_connection_id = in_connection_id;

        self.flood_all_neighbors(lsa, in_connection_id);

        if run_spf {
            self.compute_spf();
        };
        // self.compute_spf();
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
        self.add_adjacency(
            link.peer_id,
            link.connection_id,
            link.realms.clone(),
            link.cost,
        );
        self.links.insert(link.connection_id, link);
        self.flood_peer(connection_id);
        self.compute_spf();
        self.purge_unreachable();
    }

    fn add_adjacency(
        &mut self,
        peer_id: NodeId,
        connection_id: ConnectionId,
        realms: RealmList,
        cost: Cost,
    ) {
        let adjacency = Adjacency {
            peer_id,
            cost, // TODO: determine cost based on link properties
            realms,
            connection_id,
        };

        let lsa_key = LsaKey {
            origin: self.self_id,
            endpoint_id: self.self_id.0,
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
                endpoint_id: self.self_id.0,
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
                        record.updated_at = Instant::now();
                        let lsa = record.lsa.clone();
                        self.flood_all_neighbors(lsa, 0.into());
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
            .extract_if(|_k, v| *v == u16::MAX.into())
            .map(|(k, _v)| k)
            .collect();

        for id in list {
            self.db.retain(|k, _| k.origin != id);
            self.routes.purge_dead_origin(id);
        }
    }

    pub(crate) fn flood_peer(&mut self, connection_id: ConnectionId) {
        if let Some(link) = self.links.get(&connection_id) {
            for record in self.db.values() {
                if record.lsa.key.origin == link.peer_id || record.lsa.dead {
                    continue; // Don't send the peer its own LSA or dead LSAs
                }
                // let lsa_message = NodeMessage::Lsa(record.lsa.clone());
                // self.pending_tx
                //     .insert((link.connection_id, record.lsa.key.clone()), record.lsa.seq);
                if let Err(e) = link.send_lsa(record.lsa.clone()) {
                    tracing::error!(
                        "Failed to send LSA to peer {:?} over connection {:?}: {:?}",
                        link.peer_id,
                        connection_id,
                        e
                    );
                } else {
                    self.pending_tx
                        .insert((link.connection_id, record.lsa.key.clone()), record.lsa.seq);
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

    fn flood_all_neighbors(
        &mut self,
        lsa: Lsa,
        in_connection_id: ConnectionId,
        // seq: u64,
        // key: LsaKey,
    ) {
        for (out_conn_id, out_link) in self.links.iter() {
            if out_conn_id == &in_connection_id {
                continue; // Don't send back to the sender
            }
            if lsa.key.origin == out_link.peer_id {
                continue; // don't forward back to the originator
            };

            match out_link.send_lsa(lsa.clone()) {
                Ok(_) => {
                    self.pending_tx
                        .insert((out_link.connection_id, lsa.key), lsa.seq);
                    tracing::trace!(
                        "Forwarded LSA with key {:?} and seq {} to peer {:?} over connection {:?}",
                        lsa.key,
                        lsa.seq,
                        out_link.peer_id,
                        out_link.connection_id
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to forward LSA with key {:?} and seq {} to peer {:?} over connection {:?}: {:?}",
                        lsa.key,
                        lsa.seq,
                        out_link.peer_id,
                        out_link.connection_id,
                        e
                    );
                }
            }
        }
    }

    pub(crate) fn add_endpoint(&mut self, endpoint_id: EndpointId, route: &Route) {
        match self.routes.add_endpoint(endpoint_id, route) {
            Ok(effect) => match effect {
                Effects::AddLsa(endpoint_id, endpoint_info) => {
                    let body = LsaBody::Endpoint(endpoint_info);
                    let key = LsaKey {
                        origin: self.self_id,
                        endpoint_id: endpoint_id.0,
                    };
                    let lsa = Lsa {
                        key,
                        seq: 0,
                        dead: false,
                        body,
                    };
                    let record = LsaRecord {
                        lsa,
                        updated_at: Instant::now(),
                    };
                    self.flood_all_neighbors(record.lsa.clone(), 0.into());

                    self.db.insert(key, record);
                    self.request_rebuild();
                }
                Effects::UpdateLsa(endpoint_id, endpoint_info) => {
                    let record = self.db.get_mut(&LsaKey {
                        origin: self.self_id,
                        endpoint_id: endpoint_id.0,
                    });

                    if let Some(record) = record {
                        record.lsa.seq += 1;
                        record.lsa.body = LsaBody::Endpoint(endpoint_info);
                        record.updated_at = Instant::now();
                        let lsa = record.lsa.clone();
                        self.flood_all_neighbors(lsa, 0.into());
                        self.request_rebuild();
                    } else {
                        tracing::error!(
                            "Failed to update LSA for endpoint {:?}: LSA not found",
                            endpoint_id
                        );
                    }
                }
                _ => {}
            },
            Err(_e) => {
                if let Route {
                    via: ForwardTo::Local(tx),
                    ..
                } = route
                {
                    // TODO cleanup this error message
                    tx.try_send(ClientMessage::FailedRegistration(endpoint_id, "".into()))
                        .ok();
                }
            }
        }
    }
    pub(crate) fn remove_endpoint(&mut self, endpoint_id: EndpointId) {
        let effects = self.routes.remove_endpoint(endpoint_id);
        match effects {
            Effects::RemoveLsa(endpoint_id) => {
                let key = LsaKey {
                    origin: self.self_id,
                    endpoint_id: endpoint_id.0,
                };
                if let Some(mut record) = self.db.remove(&key) {
                    record.lsa.dead = true;
                    record.lsa.seq += 1;
                    self.flood_all_neighbors(record.lsa.clone(), 0.into());
                } else {
                    tracing::error!(
                        "Failed to remove LSA for endpoint {:?}: LSA not found",
                        endpoint_id
                    );
                }
            }
            _ => {}
        }
    }
}
