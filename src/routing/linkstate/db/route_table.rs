use crate::tokio;
use std::collections::HashMap;

#[cfg(feature = "remote")]
use crate::routing::NodeId;
use crate::routing::linkstate::db::route_table::RouteTableError::MismatchedRouteKind;
use crate::routing::{Cost, RouteKind, linkstate::LsRoute};
use crate::{
    EndpointId,
    messages::ClientMessage,
    routing::{
        EndpointInfo,
        linkstate::{LsForwardTo, LsRouteEntry},
    },
};
use tokio::sync::mpsc::Sender;

/// This will eventually replace the existing routing_table.rs
///

#[derive(Debug)]
pub(crate) struct RouteTable {
    table: HashMap<EndpointId, LsRouteEntry>,
}

impl RouteTable {
    pub(crate) fn routes(&self) -> &HashMap<EndpointId, LsRouteEntry> {
        &self.table
    }

    pub(crate) fn shutdown(&mut self) {
        // send a close to every Sender
        for re in self.table.values() {
            for fwd in re.routes.iter() {
                match fwd.via {
                    LsForwardTo::Local(ref sender) => {
                        sender.try_send(ClientMessage::Shutdown).ok();
                    }
                    #[cfg(feature = "remote")]
                    LsForwardTo::Remote(ref _node_id) => {}
                }
            }
        }

        self.table.clear()
    }
}

#[derive(Debug)]
pub(crate) enum Effects {
    #[cfg_attr(not(feature = "remote"), allow(unused))]
    AddLsa(EndpointId, EndpointInfo),
    #[cfg_attr(not(feature = "remote"), allow(unused))]
    UpdateLsa(EndpointId, Cost),
    #[cfg_attr(not(feature = "remote"), allow(unused))]
    RemoveLsa(EndpointId),
    RebuildFib,
    Noop,
    // UnicastAlreadyExists,
}

fn min_local_cost(routes: &[LsRoute]) -> Option<Cost> {
    routes
        .iter()
        .filter(|route| matches!(&route.via, LsForwardTo::Local(_)))
        .map(|route| route.cost)
        .min()
}

#[derive(Debug)]
pub(crate) enum RouteTableError {
    MismatchedRouteKind,
    DuplicateUnicast,
}

impl RouteTable {
    pub(crate) fn new() -> Self {
        Self {
            table: HashMap::new(),
        }
    }

    pub(crate) fn add_endpoint(
        &mut self,
        endpoint_id: EndpointId,
        endpoint_info: EndpointInfo,
        sender: Sender<ClientMessage>,
    ) -> Result<Effects, RouteTableError> {
        let mut effect = Effects::Noop;

        let route_entry = self.table.entry(endpoint_id).or_insert(LsRouteEntry {
            routes: vec![],
            kind: endpoint_info.kind,
        });
        if route_entry.kind != endpoint_info.kind {
            return Err(MismatchedRouteKind);
        }

        let ls_route = LsRoute {
            via: LsForwardTo::Local(sender),
            cost: endpoint_info.cost,
            realm: endpoint_info.realm,
            kind: endpoint_info.kind,
        };

        match endpoint_info.kind {
            RouteKind::Unicast => {
                if route_entry.routes.is_empty() {
                    effect = Effects::AddLsa(endpoint_id, endpoint_info);
                    route_entry.routes.push(ls_route);
                } else {
                    return Err(RouteTableError::DuplicateUnicast);
                }
            }
            RouteKind::Anycast => {
                let previous_min = min_local_cost(&route_entry.routes);
                let cost = endpoint_info.cost;
                route_entry.routes.push(ls_route);
                effect = match previous_min {
                    None => Effects::AddLsa(endpoint_id, endpoint_info),
                    Some(previous) if cost < previous => Effects::UpdateLsa(endpoint_id, cost),
                    Some(_) => Effects::Noop,
                };
            }
            RouteKind::Broadcast | RouteKind::Multicast => {
                if route_entry
                    .routes
                    .iter()
                    .any(|r| matches!(&r.via, LsForwardTo::Local(_sender)))
                {
                    effect = Effects::Noop;
                } else {
                    effect = Effects::AddLsa(endpoint_id, endpoint_info);
                }
                route_entry.routes.push(ls_route);
            }
            RouteKind::Node => {}
        }

        Ok(effect)
    }

    pub(crate) fn remove_endpoint(&mut self, endpoint_id: EndpointId) -> Effects {
        let Some(entry) = self.table.get_mut(&endpoint_id) else {
            return Effects::Noop;
        };
        let previous_min = min_local_cost(&entry.routes);
        let mut delete = true;
        let len = entry.routes.len();
        entry.routes.retain(|r| match r.via {
            LsForwardTo::Local(ref sender) => {
                if sender.is_closed() {
                    false
                } else {
                    delete = false;
                    true
                }
            }
            #[cfg(feature = "remote")]
            LsForwardTo::Remote(_) => {
                delete = false;
                true
            }
        });
        if delete {
            self.table.remove(&endpoint_id);
            return Effects::RemoveLsa(endpoint_id);
        }
        if entry.kind == RouteKind::Anycast {
            if let Some(cost) = min_local_cost(&entry.routes) {
                if previous_min != Some(cost) {
                    return Effects::UpdateLsa(endpoint_id, cost);
                }
            }
        }
        if len != entry.routes.len() {
            return Effects::RebuildFib;
        }
        Effects::Noop
    }

    #[cfg(feature = "remote")]

    pub(crate) fn add_remote_endpoint(
        &mut self,
        endpoint_id: EndpointId,
        origin: NodeId,
        info: EndpointInfo,
    ) -> Result<Effects, RouteTableError> {
        let mut effect = Effects::Noop;
        let route_entry = self
            .table
            .entry(endpoint_id)
            .or_insert_with(|| LsRouteEntry {
                routes: vec![],
                kind: info.kind,
            });
        if route_entry.kind != info.kind {
            return Err(RouteTableError::MismatchedRouteKind);
        }
        if let Some(existing) = route_entry
            .routes
            .iter_mut()
            .find(|r| matches!(r.via, LsForwardTo::Remote(node_id) if node_id == origin))
        {
            if existing.cost != info.cost || existing.realm != info.realm {
                existing.cost = info.cost;
                existing.realm = info.realm;
                return Ok(Effects::RebuildFib);
            }
            return Ok(Effects::Noop);
        }
        if route_entry.kind == RouteKind::Unicast && !route_entry.routes.is_empty() {
            return Err(RouteTableError::DuplicateUnicast);
        }
        if route_entry.routes.iter().all(|r| r.via.is_local()) {
            effect = Effects::RebuildFib;
        }

        route_entry.routes.push(LsRoute {
            via: LsForwardTo::Remote(origin),
            cost: info.cost,
            realm: info.realm,
            kind: info.kind,
        });
        Ok(effect)
    }

    #[cfg(feature = "remote")]

    pub(crate) fn remove_remote_endpoint(
        &mut self,
        endpoint_id: EndpointId,
        origin: NodeId,
    ) -> Effects {
        let Some(entry) = self.table.get_mut(&endpoint_id) else {
            return Effects::Noop;
        };
        let len = entry.routes.len();
        entry.routes.retain(|r| match r.via {
            LsForwardTo::Remote(ref node_id) => {
                if *node_id == origin {
                    false
                } else {
                    true
                }
            }
            LsForwardTo::Local(_) => true,
        });
        let rebuild = len != entry.routes.len();
        if entry.routes.is_empty() {
            self.table.remove(&endpoint_id);
            return Effects::RebuildFib;
        }
        if rebuild {
            Effects::RebuildFib
        } else {
            Effects::Noop
        }
    }

    // pub(crate) fn purge_dead_origin(&mut self, origin: NodeId) {
    //     let mut to_remove = vec![];
    //     for (endpoint_id, entry) in self.table.iter_mut() {
    //         let len = entry.routes.len();
    //         entry.routes.retain(|r| match r.via {
    //             LsForwardTo::Remote(ref node_id) => *node_id != origin,
    //             LsForwardTo::Local(_) => true,
    //         });
    //         if entry.routes.is_empty() {
    //             to_remove.push(*endpoint_id);
    //         } else if len != entry.routes.len() {
    //         }
    //     }
    //     for endpoint_id in to_remove {
    //         self.table.remove(&endpoint_id);
    //     }
    // }
}
