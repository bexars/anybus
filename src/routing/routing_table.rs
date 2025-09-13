use std::collections::{HashMap, HashSet};

use tracing::{debug, trace};
use uuid::Uuid;

use crate::routing::{
    Address, Advertisement, EndpointId, ForwardTo, NodeId, Realm, Route, RouteKind,
    RouteTableError, router::PeerInfo,
};

#[derive(Debug, Clone, Default)]
pub(super) struct RoutingTable {
    pub(crate) table: HashMap<EndpointId, RouteEntry>,
    pub(crate) node_id: NodeId,
    #[cfg(any(feature = "net", feature = "ipc"))]
    pub(crate) peers: HashMap<Uuid, PeerInfo>,
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

    pub(crate) fn add_peer_endpoints(
        &mut self,
        peer_id: Uuid,
        advertisements: HashSet<Advertisement>,
    ) -> Option<usize> {
        let mut routes_to_add = Vec::new();
        if let Some(peer) = self.peers.get_mut(&peer_id) {
            // let peer_tx = peer.peer_tx.clone();
            for ad in advertisements {
                use RouteKind as RK;
                let forward_to = match ad.kind {
                    RK::Unicast | RK::Anycast | RK::Node => {
                        ForwardTo::Remote(peer.peer_tx.clone(), peer_id)
                    }
                    RK::Broadcast => ForwardTo::Broadcast(vec![]),

                    RK::Multicast => {
                        let mut hs = HashSet::new();
                        hs.insert(Address::Remote(ad.endpoint_id, peer_id.into()));
                        ForwardTo::Multicast(hs)
                    }
                };
                let learned_from = match ad.kind {
                    RK::Unicast | RK::Node | RK::Anycast => peer_id,
                    RK::Multicast | RK::Broadcast => Uuid::nil().into(),
                };
                let route = Route {
                    kind: ad.kind,
                    via: forward_to,
                    learned_from,
                    realm: Realm::Global,
                    cost: ad.cost + 16,
                };

                trace!(
                    "Added route for endpoint {} via peer {}",
                    &ad.endpoint_id, &peer_id
                );
                routes_to_add.push((ad.endpoint_id.clone(), route));
                // _ = self.add_route(ad.endpoint_id.clone(), route);

                peer.received_routes.insert(ad);
            }
        } else {
            trace!("Peer {} not found for AddPeerEndpoints", peer_id);
        }
        let num_routes = routes_to_add.len();
        if num_routes == 0 {
            return None;
        }
        for (endpoint_id, route) in routes_to_add {
            _ = self.add_route(endpoint_id, route);
        }
        Some(num_routes)
    }
}

#[derive(Debug, Clone)]
pub(super) struct RouteEntry {
    // pub(crate) endpoint_id: EndpointId,
    pub(crate) routes: Vec<Route>,
    pub(crate) kind: RouteKind,
}

impl RouteEntry {
    pub(crate) fn new(kind: RouteKind) -> Self {
        let routes = Vec::new();
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
            self.routes.push(route);
            return Ok(());
        } else {
            if self.kind == RouteKind::Multicast {
                let mut orig = self.routes.pop().unwrap();
                orig.add_broadcast(route.clone());
                self.routes.push(orig);
                debug!("Broadcast list update: {:?}", self.routes);
                return Ok(());
            } else {
                let pos = self.routes.binary_search(&route).unwrap_or_else(|e| e);
                self.routes.insert(pos, route);
                return Ok(());
            }
        }

        // Ok(())
    }

    pub(crate) fn best_route(&self) -> Option<&Route> {
        self.routes.first()
    }
}
