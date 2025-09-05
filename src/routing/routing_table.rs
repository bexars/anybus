use std::collections::HashMap;

use sorted_vec::SortedVec;
use uuid::Uuid;

use crate::routing::{EndpointId, Route, RouteKind, RouteTableError};

#[derive(Debug, Clone, Default)]
pub(super) struct RoutingTable {
    pub(crate) table: HashMap<EndpointId, RouteEntry>,
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
            .or_insert_with(|| RouteEntry::new(endpoint_id, kind))
            .add_route(route)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(super) struct RouteEntry {
    pub(crate) endpoint: EndpointId,
    pub(crate) routes: SortedVec<Route>,
    pub(crate) kind: RouteKind,
}

impl RouteEntry {
    pub(crate) fn new(endpoint_id: EndpointId, kind: RouteKind) -> Self {
        let routes = SortedVec::new();
        Self {
            endpoint: endpoint_id,
            routes,
            kind,
        }
    }

    pub(crate) fn add_route(&mut self, route: Route) -> Result<(), (Route, RouteTableError)> {
        if route.kind != self.kind {
            return Err((route, RouteTableError::DifferentRouteKind(self.kind)));
        }
        self.routes.insert(route);
        Ok(())
    }

    pub(crate) fn best_route(&self) -> Option<&Route> {
        self.routes.first()
    }
}
