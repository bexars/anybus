use std::collections::HashMap;

use crate::routing::linkstate::LsRouteEntry;
#[cfg(feature = "remote")]
use crate::{
    EndpointId,
    routing::{
        Cost, ForwardTo, NodeId, Route, RouteKind,
        linkstate::{
            EndpointInfo, LsForwardTo, LsRoute,
            db::route_table::RouteTableError::MismatchedRouteKind,
        },
    },
};

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
}

pub(crate) enum Effects {
    AddLsa(EndpointId, EndpointInfo),
    UpdateLsa(EndpointId, EndpointInfo),
    RemoveLsa(EndpointId),
    RebuildFib,
    Noop,
}

#[derive(Debug)]
pub(crate) enum RouteTableError {
    MismatchedRouteKind,
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
        route: &Route,
    ) -> Result<Effects, RouteTableError> {
        let mut effect = Effects::Noop;

        let route_entry = self.table.entry(endpoint_id).or_insert(LsRouteEntry {
            routes: vec![],
            kind: route.kind,
        });
        if route_entry.kind != route.kind {
            return Err(MismatchedRouteKind);
        }

        let sender = match route.via {
            ForwardTo::Local(ref sender) => sender.clone(),
            ForwardTo::Remote(ref _sender, _connection_id) => unreachable!(),
            ForwardTo::Broadcast(ref senders, _realm) => senders[0].clone(),
            ForwardTo::Multicast(ref _hash_set) => todo!(),
        };

        let ls_route = LsRoute {
            via: LsForwardTo::Local(sender),
            cost: route.cost,
            #[cfg(feature = "remote")]
            realm: route.realm,
            #[cfg(feature = "remote")]
            kind: route.kind,
        };

        if route_entry.routes.is_empty() {
            route_entry.routes.push(ls_route);
            #[cfg(feature = "remote")]
            {
                effect = Effects::AddLsa(endpoint_id, route.into());
            }
        } else {
            let old_cheap = route_entry.routes.iter().min_by_key(|r| r.cost).unwrap();
            if old_cheap.cost > ls_route.cost {
                #[cfg(feature = "remote")]
                {
                    effect = Effects::UpdateLsa(endpoint_id, (&ls_route).into())
                };
                route_entry.routes.push(ls_route);
            }
        }

        Ok(effect)
    }

    pub(crate) fn remove_endpoint(&mut self, endpoint_id: EndpointId) -> Effects {
        let Some(entry) = self.table.get_mut(&endpoint_id) else {
            return Effects::Noop;
        };
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
            LsForwardTo::Remote(_) => {
                delete = false;
                true
            }
        });
        if delete {
            self.table.remove(&endpoint_id);
            return Effects::RemoveLsa(endpoint_id);
        }
        if len != entry.routes.len() {
            return Effects::RebuildFib;
        }
        Effects::Noop
    }

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

        if route_entry.routes.iter().all(|r| r.via.is_local()) {
            effect = Effects::RebuildFib;
        }

        let min_cost = route_entry.min_cost();
        if info.cost < min_cost
            || matches!(
                route_entry.kind,
                RouteKind::Broadcast | RouteKind::Multicast
            )
        {
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

    pub(crate) fn purge_dead_origin(&mut self, origin: NodeId) {
        let mut to_remove = vec![];
        for (endpoint_id, entry) in self.table.iter_mut() {
            let len = entry.routes.len();
            entry.routes.retain(|r| match r.via {
                LsForwardTo::Remote(ref node_id) => *node_id != origin,
                LsForwardTo::Local(_) => true,
            });
            if entry.routes.is_empty() {
                to_remove.push(*endpoint_id);
            } else if len != entry.routes.len() {
            }
        }
        for endpoint_id in to_remove {
            self.table.remove(&endpoint_id);
        }
    }
}
