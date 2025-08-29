use serde::{Deserialize, Serialize};
use sorted_vec::partial::SortedVec;
use std::collections::HashMap;
use thiserror::Error;

use tokio::sync::{mpsc::UnboundedSender, watch};
use uuid::Uuid;

use crate::{
    PeerHandle,
    messages::{ClientMessage, NodeMessage},
};

type UpdateRequired = bool;

// type Routes = HashMap<Uuid, Nexthop>;
type RoutesWatchTx = watch::Sender<Routes>;
pub(crate) type RoutesWatchRx = watch::Receiver<Routes>;

#[derive(Debug, Clone, Default)]
pub(crate) struct Routes {
    nexthops: HashMap<Uuid, Nexthop>,
    peers: HashMap<Uuid, UnboundedSender<NodeMessage>>,
}

impl Routes {
    pub(crate) fn get_route(&self, uuid: &Uuid) -> Option<&Nexthop> {
        self.nexthops.get(&uuid)
    }

    pub(crate) fn get_route_dest(&self, uuid: &Uuid) -> Option<DestinationType> {
        self.nexthops
            .get(&uuid)
            .map(|nexthop| match nexthop {
                Nexthop::Anycast(sorted_vec) => {
                    sorted_vec.first().map(|dest| &dest.dest_type).unwrap()
                }
                Nexthop::Broadcast(_sender) => todo!(),
                Nexthop::Unicast(UnicastDest {
                    unicast_type: _,
                    dest_type,
                }) => dest_type,
            })
            .cloned()
    }

    pub(crate) fn add_unicast(
        &mut self,
        uuid: Uuid,
        dest: UnicastDest,
    ) -> Result<(), RouteTableError> {
        let current_endpoint = self.nexthops.get_mut(&uuid);
        match current_endpoint {
            Some(_endpoint) => {
                return Err(RouteTableError::DuplicateRoute);
            }
            None => {
                let endpoint = Nexthop::Unicast(dest);
                self.nexthops.insert(uuid, endpoint);
            }
        }
        Ok(())
    }

    pub(crate) fn add_anycast(
        &mut self,
        uuid: Uuid,
        dest: AnycastDest,
    ) -> Result<(), RouteTableError> {
        let current_endpoint = self.nexthops.get_mut(&uuid);
        match current_endpoint {
            // No entry, insert a fresh Vector with one destination
            None => {
                let mut v = SortedVec::new();
                v.insert(dest);
                let nexthop = Nexthop::Anycast(v);
                self.nexthops.insert(uuid, nexthop);
            }

            // Entry exists, insert a new destination
            Some(Nexthop::Anycast(v)) => {
                v.insert(dest);
            }

            // Catches these two case
            //Some(Nexthop::Unicast(_))
            //Some(Nexthop::Broadcast(_))
            _ => {
                return Err(RouteTableError::DuplicateRoute);
            }
        }
        Ok(())
    }

    pub(crate) fn add_peer(&mut self, uuid: Uuid, tx: UnboundedSender<NodeMessage>) {
        self.peers.insert(uuid, tx);
    }

    pub(crate) fn dead_link(&mut self, uuid: Uuid) -> Result<UpdateRequired, RouteTableError> {
        let Some(nexthop) = self.nexthops.get_mut(&uuid) else {
            return Ok(false);
        };
        match nexthop {
            Nexthop::Broadcast(_) => todo!(),
            Nexthop::Anycast(v) => {
                let size = v.len();
                v.retain(|dest| match &dest.dest_type {
                    DestinationType::Remote(_) => true,
                    DestinationType::Local(tx) => !tx.is_closed(),
                });
                if size != v.len() {
                    // map = Arc::new(new_map);
                    return Ok(true);
                }
            }
            Nexthop::Unicast(dest) => {
                if let UnicastDest {
                    dest_type: DestinationType::Local(local),
                    ..
                } = dest
                {
                    if !local.is_closed() {
                        return Ok(false);
                    }
                };
                self.nexthops.remove(&uuid);
                return Ok(true);
            }
        };
        Ok(false)
    }

    pub(crate) fn get_peer(&self, uuid: &Uuid) -> Option<UnboundedSender<NodeMessage>> {
        self.peers.get(uuid).cloned()
    }

    pub(crate) fn shutdown_routing(&mut self) {
        for (_id, entry) in self.nexthops.iter() {
            match entry {
                Nexthop::Broadcast(_) => todo!(),
                Nexthop::Anycast(sorted_vec) => {
                    for entry in sorted_vec.iter() {
                        match &entry.dest_type {
                            DestinationType::Local(tx) => {
                                let _ = tx.send(ClientMessage::Shutdown);
                            }
                            DestinationType::Remote(_uuid) => {} //TODO inform neighbor,
                        }
                    }
                }
                Nexthop::Unicast(unicast_dest) => match &unicast_dest.dest_type {
                    DestinationType::Local(unbounded_sender) => {
                        let _ = unbounded_sender.send(ClientMessage::Shutdown);
                    }
                    DestinationType::Remote(_node) => {}
                },
            }
        }
        self.nexthops = HashMap::new();
    }
}

pub(crate) struct RouteTableController {
    routes: Routes,
    routes_watch_tx: RoutesWatchTx,
    routes_watch_rx: RoutesWatchRx,
    uuid: Uuid,
}

impl RouteTableController {
    pub(crate) fn new(uuid: Uuid) -> Self {
        let routes = Routes::default();
        let (tx, rx) = watch::channel(routes.clone());
        Self {
            routes,
            routes_watch_tx: tx,
            routes_watch_rx: rx,
            uuid,
        }
    }

    pub(crate) fn get_watcher(&self) -> RoutesWatchRx {
        self.routes_watch_rx.clone()
    }

    pub(crate) fn add_unicast(
        &mut self,
        uuid: Uuid,
        dest: UnicastDest,
    ) -> Result<(), RouteTableError> {
        self.routes.add_unicast(uuid, dest)?;
        self.update_watchers()
    }

    pub(crate) fn add_anycast(
        &mut self,
        uuid: Uuid,
        dest: AnycastDest,
    ) -> Result<(), RouteTableError> {
        //
        self.routes.add_anycast(uuid, dest)?;
        return self.update_watchers();
    }

    pub(crate) fn add_peer(
        &mut self,
        uuid: Uuid,
        tx: UnboundedSender<NodeMessage>,
    ) -> Result<(), RouteTableError> {
        self.routes.add_peer(uuid, tx);
        self.update_watchers()
    }

    pub(crate) fn dead_link(&mut self, uuid: Uuid) -> Result<(), RouteTableError> {
        let update_required = self.routes.dead_link(uuid)?;
        if update_required {
            return self.update_watchers();
        }

        Ok(())
    }

    pub(crate) fn shutdown_routing(&mut self) {
        self.routes.shutdown_routing();
        let _ = self.update_watchers();
    }

    fn update_watchers(&mut self) -> Result<(), RouteTableError> {
        if self.routes_watch_tx.send(self.routes.clone()).is_err() {
            return Err(RouteTableError::ShutdownRequested);
        }
        Ok(())
    }

    pub(crate) fn get_peer(&self, uuid: Uuid) -> Option<UnboundedSender<NodeMessage>> {
        self.routes.peers.get(&uuid).cloned()
    }

    pub(crate) fn get_advertisements(&self) -> Vec<Advertisement> {
        let mut ads = Vec::new();
        for (uuid, nexthop) in self.routes.nexthops.iter() {
            match nexthop {
                Nexthop::Anycast(sorted_vec) => {
                    for dest in sorted_vec {
                        if let AnycastDest {
                            dest_type: DestinationType::Local(_),
                            cost,
                        } = dest
                        {
                            ads.push(Advertisement::Anycast(*uuid, *cost));
                        }
                    }
                }
                Nexthop::Broadcast(sender) => todo!(),
                Nexthop::Unicast(unicast_dest) => {
                    if let UnicastDest {
                        unicast_type: UnicastType::Datagram,
                        dest_type: DestinationType::Local(_),
                    } = unicast_dest
                    {
                        ads.push(Advertisement::UnicastDatagram(*uuid))
                    }
                }
            }
        }
        ads
    }

    pub(crate) fn add_peer_advertisements(
        &mut self,
        remote_uuid: Uuid,
        advertisements: Vec<Advertisement>,
    ) {
        {
            for ad in advertisements {
                match ad {
                    Advertisement::UnicastDatagram(uuid) => {
                        let dest = UnicastDest {
                            unicast_type: UnicastType::Datagram,
                            dest_type: DestinationType::Remote(remote_uuid),
                        };
                        let _ = self.add_unicast(uuid, dest);
                    }
                    Advertisement::Anycast(uuid, cost) => {
                        let dest = AnycastDest {
                            cost: cost,
                            dest_type: DestinationType::Remote(remote_uuid),
                        };
                        let _ = self.add_anycast(uuid, dest);
                    }
                }
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum Advertisement {
    UnicastDatagram(Uuid),
    Anycast(Uuid, u16),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)] //TODO
pub(crate) enum UnicastType {
    Datagram,
    Rpc,
    RpcResponse,
}

#[derive(Debug, Clone)]
pub(crate) enum DestinationType {
    Local(UnboundedSender<ClientMessage>),
    #[allow(dead_code)] //TODO
    Remote(Uuid),
}

#[derive(Debug, Clone)]
#[allow(dead_code)] //TODO
pub(crate) struct UnicastDest {
    pub(crate) unicast_type: UnicastType,
    pub(crate) dest_type: DestinationType,
}

#[derive(Debug, Clone)]
pub(crate) struct AnycastDest {
    pub(crate) cost: u16,
    pub(crate) dest_type: DestinationType,
}

impl PartialOrd for AnycastDest {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.cost.partial_cmp(&other.cost)
    }
}

impl PartialEq for AnycastDest {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) enum Nexthop {
    // AnycastOld(SortedVec<AnycastEntry>),
    Anycast(SortedVec<AnycastDest>),
    Broadcast(tokio::sync::broadcast::Sender<ClientMessage>),
    Unicast(UnicastDest),
    // External(tokio::sync::mpsc::Sender<ClientMessage>),
}

#[derive(Debug, Error)]
pub(crate) enum RouteTableError {
    #[error("Duplicate route")]
    DuplicateRoute,
    #[error("All handles closed")]
    ShutdownRequested,
}
