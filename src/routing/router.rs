// use tokio_with_wasm::alias as tokio;

#[cfg(feature = "remote")]
use std::collections::HashSet;
use std::sync::Arc;
// use web_time::Instant;

#[cfg(feature = "remote")]
use crate::routing::{Advertisement, ConnectionId, PeerEntry};

use crate::routing::LsDb;
use crate::{
    Handle,
    routing::linkstate::{EndpointInfo, ForwardingTable},
};

use arc_swap::ArcSwap;
use tokio_with_wasm::alias as tokio;

use tokio::{
    select,
    sync::mpsc::{self},
};
use tracing::{info, trace};

use crate::{
    messages::{ClientMessage, RouterMsg},
    routing::{EndpointId, NodeId},
};

#[derive(Debug)]
pub(crate) struct Router {
    #[allow(dead_code)]
    anybus_id: NodeId,
    broker_rx: mpsc::Receiver<RouterMsg>,
    handle: Handle,
    lsdb: LsDb,
    fib: Arc<ArcSwap<ForwardingTable>>,
}

impl Router {
    pub(crate) fn new(node_id: NodeId) -> Self {
        let forward_table = ForwardingTable::new(node_id);
        let fib = Arc::new(ArcSwap::from_pointee(forward_table));

        let (broker_tx, broker_rx) = tokio::sync::mpsc::channel(32);
        let handle = Handle {
            tx: broker_tx,
            fib: Arc::clone(&fib),
            // route_watch_rx: rx,
        };

        Self {
            anybus_id: node_id,
            broker_rx,
            lsdb: LsDb::new(node_id),
            handle,
            fib: fib,
        }
    }

    pub(crate) async fn start(mut self) {
        use State::*;
        let mut next_state = Some(Start);
        while let Some(current_state) = next_state {
            trace!("Entering {:?}", &current_state);
            next_state = current_state.next(&mut self).await;
        }
    }

    pub(crate) fn get_handle(&self) -> Handle {
        self.handle.clone()
    }
}

#[derive(Debug)]
enum State {
    Start,
    Listen,
    HandleBrokerMsg(RouterMsg),
    RegisterEndpoint(
        EndpointId,
        EndpointInfo,
        tokio::sync::mpsc::Sender<ClientMessage>,
    ),
    RouteChange, // Update the Fib
    RefreshLSAs,
    Shutdown,
}

impl State {
    async fn next(self, router: &mut Router) -> Option<State> {
        use State::*;
        // dbg!(&self);
        match self {
            // ####### Start ##################################################
            Start => {
                info!("Router started");
                return Some(Listen);
            }

            // ####### Listen ##################################################
            Listen => {
                let next_tick_at = router.lsdb.when_tick();

                select! {
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
                    },
                    () = ::tokio::time::sleep_until(next_tick_at) => {
                        Some(RefreshLSAs)
                    }
                }
            }

            // ####### HandleBrokerMsg ##################################################
            HandleBrokerMsg(broker_msg) => {
                //
                match broker_msg {
                    RouterMsg::RegisterEndpoint(endpoint_id, endpoint_info, tx) => {
                        return Some(RegisterEndpoint(endpoint_id, endpoint_info, tx));
                    }
                    RouterMsg::DeadLink(endpoint_id) => {
                        router.lsdb.remove_endpoint(endpoint_id);

                        Some(RouteChange)
                    }
                    #[cfg(feature = "remote")]
                    RouterMsg::RegisterPeer(peer_id, connection_id, peer_entry, cost, realms) => {
                        let link = crate::routing::Link::new(
                            peer_entry.peer_tx.clone(),
                            peer_id,
                            connection_id,
                            realms,
                            cost,
                            false,
                            false,
                        );
                        router.lsdb.add_peer(link);
                        // dbg!(&router.lsadb);

                        return Some(RouteChange);
                    }
                    #[cfg(feature = "remote")]
                    RouterMsg::UnRegisterPeer(connection_id) => {
                        router.lsdb.remove_peer(connection_id);
                        // dbg!(&router.lsadb);

                        return Some(RouteChange);
                    }

                    #[cfg(feature = "remote")]
                    RouterMsg::LsaInbound { from, lsa } => {
                        // dbg!(&from, &lsa);
                        router.lsdb.handle_lsa(lsa, from);
                        // dbg!(&router.lsadb);
                        Some(RouteChange)
                    }

                    #[cfg(feature = "remote")]
                    RouterMsg::LsaAckInbound { from, key, seq } => {
                        router.lsdb.handle_ack(from, key, seq);
                        // dbg!(&router.lsadb);
                        Some(Listen)
                    }

                    RouterMsg::Shutdown => {
                        info!("Router shutting down");
                        return Some(Shutdown);
                    }
                }
            }

            // ####### RegisterRoute ##################################################
            RegisterEndpoint(endpoint_id, endpoint_info, sender) => {
                router.lsdb.add_endpoint(endpoint_id, endpoint_info, sender);

                Some(RouteChange)
            }

            // ####### Shutdown ##################################################
            Shutdown => {
                info!("Shutting down");
                router.lsdb.shutdown();

                return None;
            }

            // ####### RouteChange ##################################################
            RouteChange => {
                let fib_table = Arc::new(router.lsdb.build_fib());
                router.fib.swap(fib_table);
                return Some(Listen);
            }

            RefreshLSAs => {
                router.lsdb.tick();
                return Some(Listen);
            }
        }
    }
}

#[derive(Debug, Clone)]
#[cfg(feature = "remote")]
#[allow(dead_code)]
pub(crate) struct PeerInfo {
    pub(crate) peer_id: NodeId,
    pub(crate) received_routes: HashSet<Advertisement>,
    pub(crate) advertised_routes: HashSet<Advertisement>,
    pub(crate) peer_entry: PeerEntry,
    pub(crate) connection_id: ConnectionId,
}

// #[cfg(feature = "remote")]
// impl PeerInfo {
//     fn new(peer_id: NodeId, peer_entry: PeerEntry, connection_id: ConnectionId) -> Self {
//         Self {
//             peer_id,
//             received_routes: Default::default(),
//             advertised_routes: Default::default(),
//             peer_entry,
//             connection_id,
//         }
//     }
// }
