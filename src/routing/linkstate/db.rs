mod forward_table;
mod route_table;

pub(crate) use forward_table::ForwardingTable;
#[cfg(feature = "remote")]
use itertools::Itertools;
use tokio::{sync::mpsc::Sender, time::Instant};
use tokio_with_wasm::alias as tokio;

#[cfg(feature = "remote")]
use std::collections::VecDeque;
use std::{collections::HashMap, fmt::Debug, time::Duration};

// use web_time::Instant;

use crate::{
    EndpointId,
    messages::ClientMessage,
    routing::{
        NodeId,
        linkstate::{EndpointInfo, LsaKey, LsaRecord, db::route_table::RouteTable},
    },
};

#[cfg(feature = "remote")]
use crate::{
    Realm,
    routing::{
        ConnectionId, Cost, Link, RealmList, RouteKind,
        linkstate::{Adjacency, Entry, Lsa, LsaBody},
    },
};
#[cfg(feature = "remote")]
use route_table::Effects;

#[derive(Debug)]
pub(crate) struct LsDb {
    db: HashMap<LsaKey, LsaRecord>,
    self_id: NodeId,
    #[cfg(feature = "remote")]
    links: HashMap<ConnectionId, Link>,
    #[cfg(feature = "remote")]
    pending_tx: HashMap<(ConnectionId, LsaKey), u64>,
    #[cfg(feature = "remote")]
    next_hop: HashMap<NodeId, Link>, // dest → neighbor
    #[cfg(feature = "remote")]
    cost: HashMap<NodeId, Cost>,
    #[cfg(feature = "remote")]
    parent: HashMap<NodeId, NodeId>, // need spf to populate this
    routes: RouteTable,
    rebuild_requested: Option<Instant>,
    last_refresh: Instant,
    lsa_timeout: Duration,
    last_tick: Instant,
}

impl LsDb {
    pub(crate) fn new(self_id: NodeId) -> LsDb {
        let lsdb = LsDb {
            db: HashMap::new(),
            self_id,
            #[cfg(feature = "remote")]
            links: HashMap::new(),
            #[cfg(feature = "remote")]
            pending_tx: HashMap::new(),
            #[cfg(feature = "remote")]
            next_hop: HashMap::new(),
            #[cfg(feature = "remote")]
            cost: HashMap::new(),
            #[cfg(feature = "remote")]
            parent: HashMap::new(),
            routes: RouteTable::new(),
            rebuild_requested: None,
            last_refresh: Instant::now(),
            lsa_timeout: Duration::from_secs(60),
            last_tick: Instant::now(),
        };

        #[cfg(feature = "remote")]
        {
            let mut lsdb = lsdb;
            let lsa_key = LsaKey {
                origin: self_id,
                endpoint_id: self_id.0,
            };
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
            lsdb.db.insert(lsa_key, record);
            lsdb
        }
        #[cfg(not(feature = "remote"))]
        lsdb
    }

    pub(crate) fn shutdown(&mut self) {
        self.db.clear();
        #[cfg(feature = "remote")]
        self.links.clear();
        self.routes.shutdown();
    }

    pub(crate) fn when_tick(&self) -> Instant {
        self.last_tick + Duration::from_millis(100)
    }

    pub(crate) fn tick(&mut self) {
        let now = Instant::now();
        self.last_tick = now;

        if self.last_refresh + self.lsa_timeout / 3 < now {
            self.refresh_and_purge_lsas();
        }
        if self.rebuild_requested.is_some() {
            self.rebuild_requested = None;
            self.build_fib();
        }
    }

    pub(crate) fn refresh_and_purge_lsas(&mut self) {
        #[cfg(feature = "remote")]
        let mut to_flood = Vec::new();
        let now = Instant::now();

        for (_, record) in self.db.iter_mut() {
            if record.lsa.key.origin != self.self_id {
                continue;
            }
            if record.updated_at + self.lsa_timeout / 3 < now {
                record.updated_at = now;
                record.lsa.seq += 1;
                #[cfg(feature = "remote")]
                let lsa = record.lsa.clone();
                #[cfg(feature = "remote")]
                to_flood.push(lsa);
            }
        }
        #[cfg(feature = "remote")]
        to_flood
            .drain(..)
            .for_each(|lsa| self.flood_all_neighbors(lsa, None));

        #[cfg(feature = "remote")]
        if self.purge_stale_lsas() {
            self.compute_spf(self.self_id);
            self.request_rebuild();
            // self.build_fib();
        }
        self.last_refresh = Instant::now();
    }

    #[cfg(feature = "remote")]
    fn purge_stale_lsas(&mut self) -> bool {
        let mut to_purge = self
            .db
            .extract_if(|_, record| record.updated_at + self.lsa_timeout < Instant::now())
            .peekable();
        let dirty = to_purge.peek().is_some();
        let dirty = dirty
            || to_purge
                .map(|(key, _)| {
                    self.routes
                        .remove_remote_endpoint(key.endpoint_id.into(), key.origin)
                })
                .any(|effect| matches!(effect, Effects::RebuildFib));

        dirty
    }

    #[cfg(feature = "remote")]

    fn remove_dead_lsa(&mut self, lsa: Lsa) {
        assert!(lsa.dead, "removed_dead_lsa called with non-dead LSA");
        if let Some(current_lsa) = self.db.get(&lsa.key) {
            if current_lsa.lsa.seq < lsa.seq {
                self.db.remove(&lsa.key);
                #[cfg(feature = "remote")]
                self.flood_all_neighbors(lsa.clone(), None);

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
        #[cfg(feature = "remote")]
        self.routes
            .remove_remote_endpoint(lsa.key.endpoint_id.into(), lsa.key.origin);
    }

    fn request_rebuild(&mut self) {
        if self.rebuild_requested.is_none() {
            self.rebuild_requested = Some(Instant::now());
        }
    }

    #[cfg(feature = "remote")]

    fn upsert_lsa(&mut self, lsa: &Lsa) -> Result<(), ()> {
        if let Some(current_lsa) = self.db.get_mut(&lsa.key) {
            if lsa.seq > current_lsa.lsa.seq {
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
                let _ = self
                    .routes
                    .add_remote_endpoint(
                        lsa.key.endpoint_id.into(),
                        lsa.key.origin,
                        endpoint_info.clone(),
                    )
                    .map_err(|e| {
                        tracing::error!("Failed to add remote endpoint: {:?}", e);
                    });
            }
        }

        Ok(())
    }

    #[cfg(feature = "remote")]
    fn send_ack(&self, in_connection_id: ConnectionId, key: LsaKey, seq: u64) {
        let Some(in_link) = self.links.get(&in_connection_id) else {
            // tracing::error!(
            //     "Received LSA from unknown connection_id {:?}. Cannot send ack.",
            //     in_connection_id
            // );
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

    #[cfg(feature = "remote")]
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
            // tracing::warn!(
            //     "Ignoring LSA with seq {} which is not newer than current seq for key {:?}",
            //     lsa.seq,
            //     lsa.key
            // );
            return;
        };

        //
        let in_connection_id = in_connection_id;

        self.flood_all_neighbors(lsa, Some(in_connection_id));

        if run_spf {
            self.compute_spf(self.self_id);
        };
        // tracing::debug!("After LSA {:#?}", self.db);

        // tracing::debug!("After LSA routing {:#?}", self.routes);
    }

    #[cfg(feature = "remote")]
    pub(crate) fn handle_ack(&mut self, from: ConnectionId, key: LsaKey, seq: u64) {
        tracing::trace!("Got an ack from: {} key: {:?} seq: {}", &from, &key, &seq);
        if let Some(val) = self.pending_tx.remove(&(from, key)) {
            if val > seq {
                self.pending_tx.insert((from, key), val);
                tracing::warn!("Re-inserting a pending ack - our seq: {val} rcvd seq: {seq}");
            }
            return;
        } else {
            tracing::warn!("Couldn't find pending ack to remove {} {:?}", from, key);
        };
        // dbg!(&self.pending_tx);
    }

    #[cfg(feature = "remote")]
    pub(crate) fn add_peer(&mut self, link: Link) {
        let connection_id = link.connection_id;
        let Some(lsa) = self.add_adjacency(
            link.peer_id,
            link.connection_id,
            link.realms.clone(),
            link.cost,
        ) else {
            return;
        };
        self.links.insert(link.connection_id, link);
        // self.compute_spf(self.self_id);
        // self.purge_unreachable();
        // let lsa = self.db.get(&LsaKey { endpoint_id: self.sel})
        self.request_rebuild();
        self.flood_all_neighbors(lsa, Some(connection_id));
        self.flood_peer(connection_id);
    }

    #[cfg(feature = "remote")]
    fn add_adjacency(
        &mut self,
        peer_id: NodeId,
        connection_id: ConnectionId,
        realms: RealmList,
        cost: Cost,
    ) -> Option<Lsa> {
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
                    seq,
                    ..
                },
            updated_at,
        } = root_lsa
        {
            adjacencies.push(adjacency);
            *seq += 1;
            *updated_at = Instant::now();
        } else {
            tracing::error!("LSA body is not a Router type for node_id: {:?}", peer_id);
            return None;
        }
        Some(root_lsa.lsa.clone())
    }

    #[cfg(feature = "remote")]
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
                        self.flood_all_neighbors(lsa, None);
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

        self.compute_spf(self.self_id);
        // self.purge_unreachable();
    }

    // pub(crate) fn _purge_unreachable(&mut self) {
    //     let list: Vec<_> = self
    //         .cost
    //         .extract_if(|_k, v| *v == u16::MAX.into())
    //         .map(|(k, _v)| k)
    //         .collect();

    //     for id in list {
    //         self.db.retain(|k, _| k.origin != id);
    //         self.routes.purge_dead_origin(id);
    //     }
    // }

    #[cfg(feature = "remote")]
    pub(crate) fn flood_peer(&mut self, connection_id: ConnectionId) {
        if let Some(link) = self.links.get(&connection_id) {
            for record in self.db.values() {
                // if record.lsa.key.origin == link.peer_id || record.lsa.dead {
                //     continue; // Don't send the peer its own LSA or dead LSAs
                // }
                // let lsa_message = NodeMessage::Lsa(record.lsa.clone());
                // self.pending_tx
                //     .insert((link.connection_id, record.lsa.key.clone()), record.lsa.seq);
                if let Err(e) = link.send_lsa(record.lsa.clone()) {
                    use crate::routing::linkstate::LinkError;

                    if matches!(e, LinkError::RealmMismatch) {
                        continue;
                    }
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

    #[cfg(feature = "remote")]
    fn compute_spf(&mut self, root: NodeId) {
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

        // let root = self.self_id;

        for node_id in nodes.keys() {
            self.cost.insert(*node_id, u16::MAX.into());
        }
        self.cost.insert(root, Cost(0));

        let mut queue = VecDeque::new();
        queue.push_back(Entry(Cost(0), root));

        while let Some(current_entry) = queue.pop_front() {
            // dbg!(current_cost, &current_node, &queue);
            if let Some(adjacencies) = nodes.get(&current_entry.1) {
                for adjacency in adjacencies.into_iter() {
                    let next_node = adjacency.peer_id;
                    let new_cost = current_entry.0 + adjacency.cost;
                    let alt_entry = Entry(new_cost, next_node);
                    let other_entry = Entry(
                        *self.cost.get(&next_node).unwrap_or(&Cost(u16::MAX)),
                        next_node,
                    );
                    if alt_entry.better(&other_entry) {
                        self.cost.insert(next_node, new_cost);
                        self.parent.insert(next_node, current_entry.1);
                        if current_entry.1 == root {
                            self.links
                                .values()
                                .find(|link| link.peer_id == next_node)
                                .map(|link| {
                                    self.next_hop.insert(next_node, link.clone());
                                });
                        } else {
                            self.next_hop.insert(
                                next_node,
                                self.next_hop.get(&current_entry.1).unwrap().clone(),
                            );
                        }

                        // self.next_hop.insert(current_node, adjacency.node_id);
                        let entry = Entry(new_cost, next_node);
                        if let Some((pos, _)) =
                            queue.iter().find_position(|Entry(_, n)| n == &next_node)
                        {
                            queue.remove(pos);
                        };
                        let pos = queue.partition_point(|e| e.better(&entry));
                        queue.insert(pos, entry);
                    }
                }
            }
        }
        // dbg!(&self.next_hop, &self.cost, &self.parent);
    }

    #[cfg(feature = "remote")]
    fn flood_all_neighbors(
        &mut self,
        lsa: Lsa,
        in_connection_id: Option<ConnectionId>,
        // seq: u64,
        // key: LsaKey,
    ) {
        for (out_conn_id, out_link) in self.links.iter() {
            if Some(*out_conn_id) == in_connection_id {
                continue; // Don't send back to the sender
            }
            if lsa.key.origin == out_link.peer_id {
                continue; // don't forward back to the originator
            };

            let from = lsa.key.origin;
            if let Some(from_hop) = self.next_hop.get(&from) {
                if from_hop.peer_id == out_link.peer_id {
                    tracing::trace!(
                        "Not forwarding LSA to {} due to back path",
                        out_link.connection_id
                    );
                    continue;
                }
            }

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

    pub(crate) fn add_endpoint(
        &mut self,
        endpoint_id: EndpointId,
        endpoint_info: EndpointInfo,
        sender: Sender<ClientMessage>,
    ) {
        match self
            .routes
            .add_endpoint(endpoint_id, endpoint_info, sender.clone())
        {
            #[cfg_attr(not(feature = "remote"), allow(unused))]
            Ok(effect) => {
                sender
                    .try_send(ClientMessage::SuccessfulRegistration(endpoint_id))
                    .ok();
                #[cfg(feature = "remote")]
                match effect {
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
                        #[cfg(feature = "remote")]
                        self.flood_all_neighbors(record.lsa.clone(), Some(0.into()));

                        self.db.insert(key, record);
                    }
                    // Effects::UpdateLsa(endpoint_id, endpoint_info) => {
                    //     let record = self.db.get_mut(&LsaKey {
                    //         origin: self.self_id,
                    //         endpoint_id: endpoint_id.0,
                    //     });

                    //     if let Some(record) = record {
                    //         record.lsa.seq += 1;
                    //         record.lsa.body = LsaBody::Endpoint(endpoint_info);
                    //         record.updated_at = Instant::now();
                    //         let lsa = record.lsa.clone();
                    //         self.flood_all_neighbors(lsa, None);
                    //         self.request_rebuild();
                    //     } else {
                    //         tracing::error!(
                    //             "Failed to update LSA for endpoint {:?}: LSA not found",
                    //             endpoint_id
                    //         );
                    //     }
                    // }
                    _ => {
                        self.request_rebuild();
                    }
                }
            }
            Err(_e) => {
                sender
                    .try_send(ClientMessage::FailedRegistration(endpoint_id, "".into()))
                    .ok();
            }
        }
    }
    pub(crate) fn remove_endpoint(&mut self, endpoint_id: EndpointId) {
        #[cfg_attr(not(feature = "remote"), allow(unused))]
        let effects = self.routes.remove_endpoint(endpoint_id);
        self.request_rebuild();
        #[cfg(feature = "remote")]
        match effects {
            Effects::RemoveLsa(endpoint_id) => {
                let key = LsaKey {
                    origin: self.self_id,
                    endpoint_id: endpoint_id.0,
                };
                if let Some(mut record) = self.db.remove(&key) {
                    record.lsa.dead = true;
                    record.lsa.seq += 1;
                    #[cfg(feature = "remote")]
                    self.flood_all_neighbors(record.lsa.clone(), None);
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

    pub(crate) fn build_fib(&mut self) -> ForwardingTable {
        #[cfg(feature = "remote")]
        self.compute_spf(self.self_id);

        // self.purge_unreachable();
        // tracing::debug!("After SPF {:#?}", self.next_hop);
        // tracing::debug!("After SPF {:#?}", self.cost);

        ForwardingTable::build_from_db(self)
    }
}
