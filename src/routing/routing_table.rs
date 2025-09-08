use std::collections::HashMap;

use sorted_vec::SortedVec;
use tracing::debug;

use crate::routing::{EndpointId, NodeId, Route, RouteKind, RouteTableError};

#[derive(Debug, Clone, Default)]
pub(super) struct RoutingTable {
    pub(crate) table: HashMap<EndpointId, RouteEntry>,
    pub(crate) node_id: NodeId,
}

impl RoutingTable {
    pub(super) fn add_route(
        &mut self,
        endpoint_id: EndpointId,
        route: Route,
    ) -> Result<(), (Route, RouteTableError)> {
        let kind = route.kind;
        self.table
            .entry(endpoint_id)
            .or_insert_with(|| RouteEntry::new(kind))
            .add_route(route)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(super) struct RouteEntry {
    // pub(crate) endpoint_id: EndpointId,
    pub(crate) routes: SortedVec<Route>,
    pub(crate) kind: RouteKind,
}

impl RouteEntry {
    pub(crate) fn new(kind: RouteKind) -> Self {
        let routes = SortedVec::new();
        Self {
            // endpoint_id,
            routes,
            kind,
        }
    }

    pub(crate) fn add_route(&mut self, route: Route) -> Result<(), (Route, RouteTableError)> {
        if route.kind != self.kind {
            return Err((route, RouteTableError::DifferentRouteKind(self.kind)));
        }
        if !self.routes.is_empty() && route.kind == RouteKind::Unicast {
            return Err((route, RouteTableError::UnicastRouteExists));
        }
        if self.routes.is_empty() {
            self.routes.insert(route);
            return Ok(());
        } else {
            if self.kind == RouteKind::Multicast {
                let mut orig = self.routes.pop().unwrap();
                orig.add_broadcast(route.clone());
                self.routes.push(orig);
                debug!("Broadcast list update: {:?}", self.routes);
                return Ok(());
            } else {
                self.routes.insert(route);
                return Ok(());
            }
        }

        // Ok(())
    }

    pub(crate) fn best_route(&self) -> Option<&Route> {
        self.routes.first()
    }
}
