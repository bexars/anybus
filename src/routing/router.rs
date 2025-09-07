#[cfg(any(feature = "net", feature = "ipc"))]
use std::collections::{HashMap, HashSet};

#[cfg(any(feature = "net", feature = "ipc"))]
use tokio::sync::mpsc::UnboundedSender;
use tokio::{
    select,
    sync::{
        mpsc::UnboundedReceiver,
        watch::{self, Receiver, Sender},
    },
};
use tracing::{info, trace};
#[cfg(any(feature = "net", feature = "ipc"))]
use uuid::Uuid;

#[cfg(any(feature = "net", feature = "ipc"))]
use crate::routing::{Advertisement, NodeMessage};
#[cfg(any(feature = "net", feature = "ipc"))]
use crate::routing::{Realm, RouteKind};

use crate::{
    messages::{BrokerMsg, BusControlMsg, ClientMessage},
    routing::{
        Address, EndpointId, ForwardTo, ForwardingTable, NodeId, Route, routing_table::RoutingTable,
    },
};

pub(crate) type RoutesWatchRx = Receiver<ForwardingTable>;

// #[allow(dead_code)]
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
    #[cfg(any(feature = "net", feature = "ipc"))]
    peers: HashMap<Uuid, PeerInfo>,
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
            },
            #[cfg(any(feature = "net", feature = "ipc"))]
            peers: HashMap::new(),
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
    #[cfg(any(feature = "net", feature = "ipc"))]

    fn send_route_updates(&mut self) {
        trace!("Peers: {:?}", self.peers);
        for (peer_id, peer_info) in self.peers.iter_mut() {
            use std::collections::HashSet;

            trace!("routing table:{:?}", self.route_table.table);
            let mut advertisements = HashSet::new();
            for (uuid, route_entry) in self.route_table.table.iter() {
                use crate::routing::Advertisement;

                let mut advertisement = Advertisement {
                    endpoint_id: *uuid,
                    kind: route_entry.kind,
                    cost: 0,
                };
                if let Some(best_route) = route_entry.best_route() {
                    advertisement.cost = best_route.cost;
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
            peer_info.advertised_routes = advertisements.clone();

            if !withdrawn.is_empty() {
                let length = withdrawn.len();
                let msg = NodeMessage::Withdraw(withdrawn);

                if let Err(e) = peer_info.peer_tx.send(msg) {
                    trace!("Failed to send route withdrawal to peer {}: {}", peer_id, e);
                } else {
                    trace!("Sent {} route withdrawals to peer {}", peer_id, length);
                }
            }
            if !advertisements.is_empty() {
                let length = advertisements.len();
                let msg = NodeMessage::Advertise(advertisements);

                if let Err(e) = peer_info.peer_tx.send(msg) {
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
                    BrokerMsg::Subscribe(_uuid, _unbounded_sender) => todo!(),
                    BrokerMsg::DeadLink(endpoint_id) => {
                        router.route_table.table.retain(|_, route_entry| {
                            let before = route_entry.routes.len();
                            route_entry.routes.retain(|route| match &route.via {
                                ForwardTo::Local(tx) => !tx.is_closed(),
                                #[cfg(any(feature = "net", feature = "ipc"))]
                                ForwardTo::Remote(tx, _) => !tx.is_closed(),
                                ForwardTo::Broadcast(_forward_tos) => true,
                            });
                            let after = route_entry.routes.len();
                            if before != after {
                                trace!(
                                    "Removed {} dead routes for endpoint {}",
                                    before - after,
                                    endpoint_id
                                );
                            }
                            !route_entry.routes.is_empty()
                        });
                        return Some(RouteChange);
                    }
                    #[cfg(any(feature = "net", feature = "ipc"))]
                    BrokerMsg::RegisterPeer(uuid, unbounded_sender) => {
                        if router.peers.contains_key(&uuid) {
                            // Peer already registered, ignore
                            trace!("Peer {} already registered", uuid);
                            return Some(Listen);
                        }
                        let route = Route {
                            kind: RouteKind::Node,
                            via: ForwardTo::Remote(unbounded_sender.clone(), uuid),
                            learned_from: uuid,
                            realm: Realm::Process,
                            cost: 0,
                        };
                        router.route_table.add_route(uuid.into(), route).unwrap();
                        let peer_info = PeerInfo::new(uuid, unbounded_sender);
                        router.peers.insert(uuid, peer_info);
                        trace!("Registered new peer {}", uuid);

                        // router.send_route_updates();
                        return Some(RouteChange);
                    }
                    #[cfg(any(feature = "net", feature = "ipc"))]
                    BrokerMsg::UnRegisterPeer(uuid) => {
                        router.route_table.table.retain(|_, route_entry| {
                            route_entry
                                .routes
                                .retain(|route| route.learned_from != uuid);
                            !route_entry.routes.is_empty()
                        });
                        router.peers.remove(&uuid);
                        return Some(RouteChange);
                    }
                    #[cfg(any(feature = "net", feature = "ipc"))]
                    BrokerMsg::AddPeerEndpoints(peer_id, hash_set) => {
                        if let Some(peer) = router.peers.get_mut(&peer_id) {
                            for ad in hash_set {
                                use RouteKind as RK;

                                let forward_to = match ad.kind {
                                    RK::Unicast | RK::Anycast | RK::Node => {
                                        ForwardTo::Remote(peer.peer_tx.clone(), peer_id)
                                    }
                                    RouteKind::Multicast => {
                                        let mut hs = HashSet::new();
                                        hs.insert(Address::Remote(ad.endpoint_id, peer_id.into()));
                                        ForwardTo::Broadcast(hs)
                                    }
                                };

                                let route = Route {
                                    kind: ad.kind,
                                    via: forward_to,
                                    learned_from: peer_id,
                                    realm: Realm::Process,
                                    cost: ad.cost + 16,
                                };

                                _ = router.route_table.add_route(ad.endpoint_id.clone(), route);

                                trace!(
                                    "Added route for endpoint {} via peer {}",
                                    ad.endpoint_id, peer_id
                                );
                                peer.received_routes.insert(ad);
                            }
                            return Some(RouteChange);
                        } else {
                            trace!("Peer {} not found for AddPeerEndpoints", peer_id);
                        }
                        return Some(Listen);
                    }
                    #[cfg(any(feature = "net", feature = "ipc"))]
                    BrokerMsg::RemovePeerEndpoints(_uuid, _uuids) => todo!(),
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
                            ForwardTo::Remote(_unbounded_sender, _node_id) => {
                                panic!("Can't create remote endpoint locally")
                            }
                            ForwardTo::Broadcast(items) => {
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
                #[cfg(any(feature = "net", feature = "ipc"))]
                for peer in router.peers.values() {
                    _ = peer.peer_tx.send(NodeMessage::Close);
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
                                #[cfg(any(feature = "net", feature = "ipc"))]
                                ForwardTo::Remote(_tx, _node_id) => {}
                                ForwardTo::Broadcast(_forward_tos) => {}
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
                #[cfg(any(feature = "net", feature = "ipc"))]
                router.send_route_updates();

                return Some(Listen);
            } // ####### HandleError ##################################################
              // HandleError(_e) => {
              //     todo!()
              // }
        };
        // #[allow(unreachable_code)]
        // unreachable!();
        // error!("Fell through the brokermsg");
        // return None;
    }
}

#[derive(Debug)]
#[allow(dead_code)]
#[cfg(any(feature = "net", feature = "ipc"))]
struct PeerInfo {
    peer_id: NodeId,
    peer_tx: UnboundedSender<NodeMessage>,
    received_routes: HashSet<Advertisement>,
    advertised_routes: HashSet<Advertisement>,
}
#[cfg(any(feature = "net", feature = "ipc"))]

impl PeerInfo {
    fn new(peer_id: NodeId, peer_tx: UnboundedSender<NodeMessage>) -> Self {
        Self {
            peer_id,
            peer_tx,
            received_routes: Default::default(),
            advertised_routes: Default::default(),
        }
    }
}
