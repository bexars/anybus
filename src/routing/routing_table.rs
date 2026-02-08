use std::collections::HashMap;
#[cfg(feature = "remote")]
use std::collections::HashSet;

use tracing::debug;
#[cfg(feature = "remote")]
use tracing::trace;
#[cfg(feature = "remote")]
use uuid::Uuid;

#[cfg(feature = "remote")]
use crate::routing::{Address, Advertisement, ForwardTo, Realm, router::PeerInfo};
use crate::routing::{EndpointId, NodeId, Route, RouteKind, RouteTableError};

#[derive(Clone, Default)]
pub(super) struct RoutingTable {
    pub(crate) table: HashMap<EndpointId, RouteEntry>,
    pub(crate) node_id: NodeId,
    #[cfg(feature = "remote")]
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

    #[cfg(feature = "remote")]
    pub(crate) fn add_peer_endpoints(
        &mut self,
        peer_id: Uuid,
        advertisements: HashSet<Advertisement>,
    ) -> Option<usize> {
        let mut routes_to_add = Vec::new();
        if let Some(peer) = self.peers.get_mut(&peer_id) {
            for ad in advertisements {
                use RouteKind as RK;
                let forward_to = match ad.kind {
                    RK::Unicast | RK::Anycast | RK::Node => {
                        ForwardTo::Remote(peer.peer_entry.peer_tx.clone(), peer_id)
                    }
                    RK::Broadcast => ForwardTo::Broadcast(vec![], Realm::Global), //TODO fix realm

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

impl std::fmt::Debug for RoutingTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.table
            .iter()
            .try_for_each(|(k, v)| -> std::fmt::Result {
                write!(f, "{k}: ")?;
                writeln!(f, "kind: {} ", v.kind)?;
                v.routes.iter().try_for_each(|r| -> std::fmt::Result {
                    #[cfg(feature = "remote")]
                    {
                        writeln!(
                            f,
                            "   cost:{:<4?} kind: {:<10} frm:{:?} rlm:{:<8?} fwd:{:<10?}",
                            r.cost, r.kind, r.learned_from, r.realm, r.via
                        )
                    }
                    #[cfg(not(feature = "remote"))]
                    {
                        writeln!(
                            f,
                            "   cost:{:<4?} kind: {:<10} fwd:{:<10?}",
                            r.cost, r.kind, r.via
                        )
                    }
                })
            })?;
        #[cfg(feature = "remote")]
        self.peers
            .iter()
            .try_for_each(|(k, v)| -> std::fmt::Result {
                writeln!(f, "Peer: {} {:?}", k, v.peer_entry.realm)?;
                writeln!(f, "Advertised")?;
                v.advertised_routes.iter().try_for_each(|r| {
                    writeln!(
                        f,
                        "-  {} cost {:?} kind {:?}",
                        r.endpoint_id, r.cost, r.kind
                    )
                })?;
                writeln!(f, "Received")?;
                v.received_routes.iter().try_for_each(|r| {
                    writeln!(
                        f,
                        "-  {} cost {:?} kind {:?}",
                        r.endpoint_id, r.cost, r.kind
                    )
                })
            })?;
        Ok(())
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
