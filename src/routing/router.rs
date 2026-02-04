use tokio_with_wasm::alias as tokio;

use std::collections::{HashMap, HashSet};

#[cfg(feature = "remote")]
use crate::routing::{Advertisement, NodeMessage};
#[cfg(feature = "remote")]
use crate::routing::{PeerEntry, Realm, RouteKind};
use tokio::{
    select,
    sync::{
        mpsc::UnboundedReceiver,
        watch::{self, Receiver, Sender},
    },
};
use tracing::{info, trace};

use crate::{
    messages::{BrokerMsg, BusControlMsg, ClientMessage},
    routing::{
        Address, EndpointId, ForwardTo, ForwardingTable, NodeId, Route, routing_table::RoutingTable,
    },
};

pub(crate) type RoutesWatchRx = Receiver<ForwardingTable>;

#[derive(Debug)]
pub(crate) struct Router {
    forward_table: ForwardingTable,
    route_table: RoutingTable,
    routes_watch_tx: Sender<ForwardingTable>,
    routes_watch_rx: Receiver<ForwardingTable>,
    #[allow(dead_code)]
    anybus_id: NodeId,
    // received_routes: HashMap<Uuid, usize>,
    // sent_routes: HashMap<Uuid, usize>,
    bus_control_rx: Receiver<BusControlMsg>,
    broker_rx: UnboundedReceiver<BrokerMsg>,
}

impl Router {
    pub(crate) fn new(
        uuid: NodeId,
        broker_rx: UnboundedReceiver<BrokerMsg>,
        bus_control_rx: Receiver<BusControlMsg>,
    ) -> Self {
        let forward_table = ForwardingTable::default();
        let (tx, rx) = watch::channel(forward_table.clone());
        Self {
            forward_table,
            routes_watch_tx: tx,
            routes_watch_rx: rx,
            anybus_id: uuid,
            // received_routes: HashMap::new(),
            // sent_routes: HashMap::new(),
            bus_control_rx,
            broker_rx,
            route_table: RoutingTable {
                table: HashMap::new(),
                node_id: uuid,
                #[cfg(feature = "remote")]
                peers: HashMap::new(),
            },
        }
    }

    pub(crate) async fn start(mut self) {
        use State::*;
        let mut current_state = Some(Start);
        while let Some(next_state) = current_state {
            trace!("Entering {:?}", &next_state);
            current_state = next_state.next(&mut self).await;
        }
    }

    pub(crate) fn get_watcher(&self) -> RoutesWatchRx {
        self.routes_watch_rx.clone()
    }
    #[cfg(feature = "remote")]

    fn send_route_updates(&mut self) {
        trace!("Peers: {:?}", self.route_table.peers);
        for (peer_id, peer_info) in self.route_table.peers.iter_mut() {
            use std::collections::HashSet;

            trace!("routing table:{:#?}", self.route_table.table);
            let mut advertisements = HashSet::new();
            for (uuid, route_entry) in self.route_table.table.iter() {
                use crate::routing::Advertisement;

                let mut advertisement = Advertisement {
                    endpoint_id: *uuid,
                    kind: route_entry.kind,
                    cost: 0,
                };
                if let Some(best_route) = route_entry.best_route() {
                    advertisement.cost = best_route.cost + 5;
                    // Don't send routes back to the peer we learned them from
                    if best_route.learned_from == *peer_id {
                        continue;
                    }
                    // Don't send routes the peer already learned from us
                    if peer_info.advertised_routes.contains(&advertisement) {
                        continue;
                    }
                    // Don't send local routes to peers
                    if best_route.realm == Realm::Process {
                        continue;
                    }
                    advertisements.insert(advertisement);
                }
            }
            trace!(
                "Peer {}: {} new advertisements to send",
                peer_id,
                advertisements.len()
            );
            let withdrawn: HashSet<_> = peer_info
                .advertised_routes
                .difference(&peer_info.advertised_routes)
                .cloned()
                .collect();
            // peer_info.advertised_routes = advertisements.clone();
            let ads: Vec<_> = advertisements
                .difference(&peer_info.advertised_routes)
                .cloned()
                .collect();
            for ad in ads {
                peer_info.advertised_routes.insert(ad);
            }
            if !withdrawn.is_empty() {
                let length = withdrawn.len();
                let msg = NodeMessage::Withdraw(withdrawn);

                if let Err(e) = peer_info.peer_entry.peer_tx.send(msg) {
                    trace!("Failed to send route withdrawal to peer {}: {}", peer_id, e);
                } else {
                    trace!("Sent {} route withdrawals to peer {}", peer_id, length);
                }
            }
            if !advertisements.is_empty() {
                let length = advertisements.len();
                let msg = NodeMessage::Advertise(advertisements);

                if let Err(e) = peer_info.peer_entry.peer_tx.send(msg) {
                    trace!(
                        "Failed to send route advertisement to peer {}: {}",
                        peer_id, e
                    );
                } else {
                    trace!("Sent {} route advertisements to peer {}", peer_id, length);
                }
            }
        }
    }
}

#[derive(Debug)]
enum State {
    Start,
    Listen,
    HandleBrokerMsg(BrokerMsg),
    RegisterRoute(EndpointId, Route),
    RouteChange, // Notify peers of route changes
    Shutdown,
    // HandleError(RouteTableError),
}

impl State {
    async fn next(self, router: &mut Router) -> Option<State> {
        use State::*;
        match self {
            // ####### Start ##################################################
            Start => {
                info!("Router started");
                return Some(Listen);
            }

            // ####### Listen ##################################################
            Listen => {
                select! {
                    res = router.bus_control_rx.changed() => {
                        trace!("bus_control_rx.changed() = {:?}", res);
                        if res.is_err() { return Some(Shutdown)}
                        match *router.bus_control_rx.borrow_and_update() {
                            BusControlMsg::Run => {
                                trace!("Router received Run");
                            }
                            BusControlMsg::Shutdown => {
                                info!("Router shutting down");
                                return Some(Shutdown);
                            }
                        }
                        return Some(Listen);
                    }
                    msg = router.broker_rx.recv() => {
                        match msg {
                            None => {
                                info!("Broker channel closed, shutting down router");
                                return Some(Shutdown);
                            }
                            Some(msg) => {
                                trace!("Router received BrokerMsg: {:?}", msg);
                                return Some(HandleBrokerMsg(msg));
                            }
                        }
                    }
                }
            }

            // ####### HandleBrokerMsg ##################################################
            HandleBrokerMsg(broker_msg) => {
                //
                match broker_msg {
                    // BrokerMsg::RegisterAnycast(uuid, unbounded_sender) => todo!(),
                    // BrokerMsg::RegisterUnicast(uuid, unbounded_sender, unicast_type) => todo!(),
                    BrokerMsg::RegisterRoute(endpoint_id, route) => {
                        return Some(RegisterRoute(endpoint_id, route));
                    }
                    BrokerMsg::DeadLink(endpoint_id) => {
                        let route_entry = router.route_table.table.get_mut(&endpoint_id);
                        if let Some(route_entry) = route_entry {
                            let before = route_entry.routes.len();
                            route_entry.routes.retain_mut(|route| match route.via {
                                ForwardTo::Local(ref tx) => !tx.is_closed(),
                                #[cfg(feature = "remote")]
                                ForwardTo::Remote(ref tx, _) => !tx.is_closed(),
                                ForwardTo::Multicast(ref _forward_tos) => true,
                                ForwardTo::Broadcast(ref mut senders, _) => {
                                    senders.retain(|sender| !sender.is_closed());
                                    // once a broadcast endpoint, always a broadcast endpoint
                                    true
                                }
                            });

                            let after = route_entry.routes.len();
                            if before != after {
                                trace!(
                                    "Removed {} dead routes for endpoint {}",
                                    before - after,
                                    endpoint_id
                                );
                                return Some(RouteChange);
                            }
                        }
                        return Some(Listen);
                    }
                    #[cfg(feature = "remote")]
                    BrokerMsg::RegisterPeer(uuid, peer_entry) => {
                        if router.route_table.peers.contains_key(&uuid) {
                            // Peer already registered, ignore
                            trace!("Peer {} already registered", uuid);
                            return Some(Listen);
                        }
                        let route = Route {
                            kind: RouteKind::Node,
                            via: ForwardTo::Remote(peer_entry.peer_tx.clone(), uuid),
                            learned_from: uuid,
                            realm: Realm::Global,
                            cost: 0,
                        };
                        router.route_table.add_route(uuid.into(), route).unwrap();
                        let peer_info = PeerInfo::new(uuid, peer_entry);
                        router.route_table.peers.insert(uuid, peer_info);
                        trace!("Registered new peer {}", uuid);

                        // router.send_route_updates();
                        return Some(RouteChange);
                    }
                    #[cfg(feature = "remote")]
                    BrokerMsg::UnRegisterPeer(uuid) => {
                        router.route_table.table.retain(|_, route_entry| {
                            route_entry
                                .routes
                                .retain(|route| route.learned_from != uuid);
                            !route_entry.routes.is_empty()
                        });
                        router.route_table.peers.remove(&uuid);
                        return Some(RouteChange);
                    }
                    #[cfg(feature = "remote")]
                    BrokerMsg::AddPeerEndpoints(peer_id, hash_set) => {
                        match router.route_table.add_peer_endpoints(peer_id, hash_set) {
                            Some(count) => {
                                trace!("Added {} routes from peer {}", count, peer_id);
                                return Some(RouteChange);
                            }
                            None => {
                                trace!("Peer {} not found for AddPeerEndpoints", peer_id);
                                return Some(Listen);
                            }
                        }
                    }
                    //     if let Some(peer) = router.route_table.peers.get_mut(&peer_id) {
                    //         for ad in hash_set {
                    //             use RouteKind as RK;

                    //             let forward_to = match ad.kind {
                    //                 RK::Unicast | RK::Anycast | RK::Node => {
                    //                     ForwardTo::Remote(peer.peer_tx.clone(), peer_id)
                    //                 }
                    //                 RK::Broadcast => ForwardTo::Broadcast(vec![]),

                    //                 RK::Multicast => {
                    //                     let mut hs = HashSet::new();
                    //                     hs.insert(Address::Remote(ad.endpoint_id, peer_id.into()));
                    //                     ForwardTo::Multicast(hs)
                    //                 }
                    //             };
                    //             let learned_from = match ad.kind {
                    //                 RK::Unicast | RK::Node | RK::Anycast => peer_id,
                    //                 RK::Multicast | RK::Broadcast => Uuid::nil().into(),
                    //             };
                    //             let route = Route {
                    //                 kind: ad.kind,
                    //                 via: forward_to,
                    //                 learned_from,
                    //                 realm: Realm::Process,
                    //                 cost: ad.cost + 16,
                    //             };

                    //             _ = router.route_table.add_route(ad.endpoint_id.clone(), route);

                    //             trace!(
                    //                 "Added route for endpoint {} via peer {}",
                    //                 ad.endpoint_id, peer_id
                    //             );
                    //             peer.received_routes.insert(ad);
                    //         }
                    //         return Some(RouteChange);
                    //     } else {
                    //         trace!("Peer {} not found for AddPeerEndpoints", peer_id);
                    //     }
                    //     return Some(Listen);
                    // }
                    #[cfg(feature = "remote")]
                    BrokerMsg::RemovePeerEndpoints(_uuid, _uuids) => return Some(Listen),
                    BrokerMsg::Shutdown => {
                        info!("Router shutting down");
                        return Some(Shutdown);
                    }
                }
            }

            // ####### RegisterRoute ##################################################
            RegisterRoute(endpoint_id, route) => {
                let forward_to = route.via.clone();
                return match router.route_table.add_route(endpoint_id, route) {
                    Ok(_) => {
                        match forward_to {
                            ForwardTo::Local(tx) => {
                                _ = tx.send(ClientMessage::SuccessfulRegistration(endpoint_id))
                            }
                            #[cfg(feature = "remote")]
                            ForwardTo::Remote(_unbounded_sender, _node_id) => {
                                panic!("Can't create remote endpoint locally")
                            }
                            ForwardTo::Broadcast(listener, _) => {
                                for tx in listener {
                                    // There should only be one in here on creation
                                    _ = tx.send(ClientMessage::SuccessfulRegistration(endpoint_id));
                                }
                            }
                            ForwardTo::Multicast(items) => {
                                match items
                                    .into_iter()
                                    .next()
                                    .expect("This should be set when this was created")
                                {
                                    Address::Endpoint(local_endpoint_id) => {
                                        // this is all to lookup the right address to return Success to
                                        if let ForwardTo::Local(tx) = router
                                            .forward_table
                                            .lookup(&local_endpoint_id.into())
                                            .unwrap()
                                        {
                                            _ = tx.send(ClientMessage::SuccessfulRegistration(
                                                endpoint_id,
                                            ));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }

                        Some(RouteChange)
                    }

                    Err((route, e)) => {
                        if let Route {
                            via: ForwardTo::Local(tx),
                            ..
                        } = route
                        {
                            _ = tx.send(ClientMessage::FailedRegistration(
                                endpoint_id,
                                e.to_string(),
                            ));
                        }
                        Some(Listen)
                    }
                };
            }

            // ####### Shutdown ##################################################
            Shutdown => {
                info!("Shutting down");
                #[cfg(feature = "remote")]
                for peer in router.route_table.peers.values() {
                    _ = peer.peer_entry.peer_tx.send(NodeMessage::Close);
                }
                router
                    .route_table
                    .table
                    .iter()
                    .for_each(|(_, route_entry)| {
                        route_entry
                            .routes
                            .iter()
                            .for_each(|route| match &route.via {
                                ForwardTo::Local(tx) => {
                                    let _ = tx.send(ClientMessage::Shutdown);
                                }
                                #[cfg(feature = "remote")]
                                ForwardTo::Remote(_tx, _node_id) => {}
                                ForwardTo::Multicast(_forward_tos) => {}
                                ForwardTo::Broadcast(listeners, _) => {
                                    for listener in listeners {
                                        let _ = listener.send(ClientMessage::Shutdown);
                                    }
                                }
                            });
                    });
                return None;
            }

            // ####### RouteChange ##################################################
            RouteChange => {
                let new_forward_table = ForwardingTable::from(&router.route_table);

                router.forward_table = new_forward_table;
                // trace!("Updated forwarding table: Router: {:#?}", router);
                let forward_table = router.forward_table.clone();
                trace!("New forwarding table: {:#?}", forward_table);

                let _ = router.routes_watch_tx.send(forward_table).unwrap();
                // trace!("Updated forwarding table: Router: {:#?}", router);
                // Notify peers of route changes
                #[cfg(feature = "remote")]
                router.send_route_updates();

                return Some(Listen);
            } // ####### HandleError ##################################################
              // HandleError(_e) => {
              //     todo!()
              // }
        }
        // #[allow(unreachable_code)]
        // unreachable!();
        // error!("Fell through the brokermsg");
        // return None;
    }
}

#[derive(Debug, Clone)]
#[cfg(feature = "remote")]
#[allow(dead_code)]
pub(crate) struct PeerInfo {
    pub(crate) peer_id: NodeId,
    // pub(crate) peer_tx: UnboundedSender<NodeMessage>,
    pub(crate) received_routes: HashSet<Advertisement>,
    pub(crate) advertised_routes: HashSet<Advertisement>,
    // pub(crate) realm: Realm,
    pub(crate) peer_entry: PeerEntry,
}
#[cfg(feature = "remote")]

impl PeerInfo {
    fn new(peer_id: NodeId, peer_entry: PeerEntry) -> Self {
        Self {
            peer_id,
            // peer_tx,
            received_routes: Default::default(),
            advertised_routes: Default::default(),
            // realm,
            peer_entry,
        }
    }
}
