use std::collections::{HashMap, HashSet};

use tokio::{
    select,
    sync::{
        mpsc::{UnboundedReceiver, UnboundedSender},
        watch::{self, Receiver, Sender},
    },
};
use tracing::{info, trace};
use uuid::Uuid;

use crate::{
    messages::{BrokerMsg, BusControlMsg, ClientMessage, NodeMessage},
    routing::{
        Advertisement, EndpointId, ForwardTo, ForwardingTable, NodeId, Realm, Route, RouteKind,
        routing_table::RoutingTable,
    },
};

pub(crate) type RoutesWatchRx = Receiver<ForwardingTable>;

#[allow(dead_code)]
pub(crate) struct Router {
    forward_table: ForwardingTable,
    route_table: RoutingTable,
    routes_watch_tx: Sender<ForwardingTable>,
    routes_watch_rx: Receiver<ForwardingTable>,
    msgbus_id: Uuid,
    received_routes: HashMap<Uuid, usize>,
    sent_routes: HashMap<Uuid, usize>,
    bus_control_rx: Receiver<BusControlMsg>,
    broker_rx: UnboundedReceiver<BrokerMsg>,
    peers: HashMap<Uuid, PeerInfo>,
}

impl Router {
    pub(crate) fn new(
        uuid: Uuid,
        broker_rx: UnboundedReceiver<BrokerMsg>,
        bus_control_rx: Receiver<BusControlMsg>,
    ) -> Self {
        let forward_table = ForwardingTable::default();
        let (tx, rx) = watch::channel(forward_table.clone());
        Self {
            forward_table,
            routes_watch_tx: tx,
            routes_watch_rx: rx,
            msgbus_id: uuid,
            received_routes: HashMap::new(),
            sent_routes: HashMap::new(),
            bus_control_rx,
            broker_rx,
            route_table: Default::default(),
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

    fn send_route_updates(&self) {
        trace!("Peers: {:?}", self.peers);
        for (peer_id, peer_info) in self.peers.iter() {
            trace!("routing table:{:?}", self.route_table.table);
            let mut advertisements = HashSet::new();
            for (uuid, route_entry) in self.route_table.table.iter() {
                if let Some(best_route) = route_entry.best_route() {
                    // Don't send routes back to the peer we learned them from
                    if best_route.learned_from == *peer_id {
                        continue;
                    }
                    // Don't send routes the peer already knows about
                    if peer_info.advertised_routes.contains_key(uuid) {
                        continue;
                    }
                    // Don't send local routes to peers
                    if best_route.realm == Realm::Process {
                        continue;
                    }
                    advertisements.insert(Advertisement {
                        endpoint_id: *uuid,
                        kind: route_entry.kind,
                        cost: best_route.cost,
                    });
                }
            }
            trace!(
                "Peer {}: {} new advertisements to send",
                peer_id,
                advertisements.len()
            );
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
                        match *router.bus_control_rx.borrow() {
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
                                ForwardTo::Remote(tx) => !tx.is_closed(),
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
                    BrokerMsg::RegisterPeer(uuid, unbounded_sender) => {
                        if router.peers.contains_key(&uuid) {
                            // Peer already registered, ignore
                            trace!("Peer {} already registered", uuid);
                            return Some(Listen);
                        }
                        let route = Route {
                            kind: RouteKind::Node,
                            via: ForwardTo::Remote(unbounded_sender.clone()),
                            learned_from: uuid,
                            realm: Realm::Process,
                            cost: 0,
                        };
                        router.route_table.add_route(uuid, route).unwrap();
                        let peer_info = PeerInfo::new(uuid, unbounded_sender);
                        router.peers.insert(uuid, peer_info);
                        router.send_route_updates();
                        trace!("Registered new peer {}", uuid);
                        return Some(Listen);
                    }
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
                    BrokerMsg::AddPeerEndpoints(uuid, hash_set) => {
                        if let Some(peer) = router.peers.get_mut(&uuid) {
                            for ad in hash_set {
                                let route = Route {
                                    kind: ad.kind,
                                    via: ForwardTo::Remote(peer.peer_tx.clone()),
                                    learned_from: uuid,
                                    realm: Realm::Process,
                                    cost: ad.cost + 16,
                                };

                                peer.received_routes.insert(ad.endpoint_id, route.clone());
                                _ = router.route_table.add_route(ad.endpoint_id, route);
                                trace!(
                                    "Added route for endpoint {} via peer {}",
                                    ad.endpoint_id, uuid
                                );
                            }
                            return Some(RouteChange);
                        } else {
                            trace!("Peer {} not found for AddPeerEndpoints", uuid);
                        }
                        return Some(Listen);
                    }
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
                        if let ForwardTo::Local(tx) = forward_to {
                            _ = tx.send(ClientMessage::SuccessfulRegistration(endpoint_id))
                        };

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
                // for forward_to in router.forward_table.table.values() {
                //     match forward_to {
                //         ForwardTo::Local(tx) => {
                //             let _ = tx.send(ClientMessage::Shutdown);
                //         }
                //         ForwardTo::Remote(tx) => {
                //             // let _ = tx.send(NodeMessage::Close);
                //         }
                //         ForwardTo::Broadcast(_forward_tos) => {}
                //     }
                // }
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
                                ForwardTo::Remote(_tx) => {}
                                ForwardTo::Broadcast(_forward_tos) => {}
                            });
                    });
                return None;
            }

            // ####### RouteChange ##################################################
            RouteChange => {
                let new_forward_table = ForwardingTable::from(&router.route_table);

                router.forward_table = new_forward_table;
                let _ = router.routes_watch_tx.send(router.forward_table.clone());
                // Notify peers of route changes
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
struct PeerInfo {
    peer_id: NodeId,
    peer_tx: UnboundedSender<NodeMessage>,
    received_routes: HashMap<Uuid, Route>,
    advertised_routes: HashMap<Uuid, Route>,
}

impl PeerInfo {
    fn new(peer_id: NodeId, peer_tx: UnboundedSender<NodeMessage>) -> Self {
        Self {
            peer_id,
            peer_tx,
            received_routes: HashMap::new(),
            advertised_routes: HashMap::new(),
        }
    }
}
