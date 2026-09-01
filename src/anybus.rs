pub(crate) mod builder;
pub(crate) mod config;
pub(crate) mod watcher;

pub use config::AnyBusConfig;

// use uuid::Uuid;

use crate::anybus::builder::AnyBusBuilder;
use crate::errors::AnyBusHandleError;
use crate::routing::NodeId;
use crate::services::BusStopService;
use crate::{
    BusDepot, BusDeserialize, BusRider, BusRiderRpc, BusRiderWithUuid, BusStop, BusStopId,
    EndpointId, spawn,
};
use crate::{Handle, routing::router::Router};
#[cfg(feature = "ws")]
use crate::{localrpc, peers};

// type RoutesWatchRx = watch::Receiver<Routes>;

/// The main entry point into the AnyBus system.
#[allow(dead_code)]
#[derive(Debug)]
pub struct AnyBus {
    id: NodeId,
    handle: Handle,
    // options: AnyBusBuilder,
    config: AnyBusConfig,
    router: Option<Router>,

    #[cfg(feature = "ws")]
    ws_rpc_client: Option<localrpc::LocalRpcClient<peers::ws::WsRpcMessage>>,
}

impl AnyBus {
    /// This starts and runs the AnyBus.
    ///
    /// The returned [BusControlHandle] is used to shutdown the system.  The [Handle] is
    /// used for normal interaction with the system
    ///
    //
    pub fn new() -> AnyBus {
        Self::init(AnyBusBuilder::default())
    }

    /// Returns a new AnyBusBuilder to configure and build an AnyBus instance
    pub fn build() -> AnyBusBuilder {
        AnyBusBuilder::new()
    }

    pub(crate) fn init_from_config(config: AnyBusConfig) -> AnyBus {
        tracing::info!("Initializing AnyBus");
        let id = NodeId::new();
        let router = Router::new(id);

        let handle = router.get_handle();

        let anybus = AnyBus {
            id,
            handle: handle.clone(),
            config,
            router: Some(router),
            #[cfg(feature = "ws")]
            ws_rpc_client: None,
        };
        anybus
    }

    pub(crate) fn init(options: AnyBusBuilder) -> AnyBus {
        let config = options.into();
        tracing::trace!("{config:?}");
        AnyBus::init_from_config(config)
    }

    /// Passes the shutdown command to the AnyBus system and all local listeners.  Immediately withdraws all advertisements from the network.
    ///
    /// If the program is killed by other means it can take up to 40 seconds for other systems to forget the advertisements from this AnyBus
    ///
    pub fn shutdown(&mut self) {
        self.handle.shutdown();
    }

    /// Returns a Handle for clients to interact with the AnyBus system.
    /// Expected to be cloned and sent to other parts of your program
    ///
    pub fn handle(&self) -> &Handle {
        &self.handle
    }

    // #[cfg(feature = "tokio")]
    /// Convenience function to spawn a task that will listen for Ctrl-C from the terminal and trigger a shutdown of the AnyBus system
    #[cfg(not(target_arch = "wasm32"))]
    pub fn shutdown_with_ctrlc(&self) {
        use crate::{helper, spawn};

        _ = spawn(helper::watch_ctrlc(self.handle.clone()));
    }

    #[cfg(feature = "ws")]
    fn start_ws_manager(&mut self) {
        let id = self.id;
        let handle = self.handle.clone();
        #[cfg(feature = "ws_server")]
        let ws_listener_options = self.config.ws_server.clone();
        let ws_peers = self
            .config
            .peer
            .iter()
            .filter_map(|(name, peer)| {
                #[allow(irrefutable_let_patterns)] // remove when there's a 2nd peer type
                if let config::PeerType::WebSocket { url } = peer {
                    use crate::anybus::config::WebSocketPeerConfig;

                    Some(WebSocketPeerConfig {
                        url: url.clone(),
                        name: name.clone(),
                    })
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let (ws_rpc_client, ws_rpc_rx) = localrpc::create_rpc::<peers::ws::WsRpcMessage>();
        self.ws_rpc_client = Some(ws_rpc_client);
        spawn(async move {
            let ws_listener = crate::peers::WebsocketManager::new(
                id,
                handle,
                // self.bc_rx.clone(),
                #[cfg(feature = "ws_server")]
                ws_listener_options,
                ws_peers,
                ws_rpc_rx,
            )
            .await;
            ws_listener.start().await
        });
    }

    /// Starts the AnyBus system.  This will start any configured listeners (WebSocket, IPC, etc) and begin processing messages.
    pub fn run(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.config.enable_ctrlc_shutdown {
            self.shutdown_with_ctrlc();
        }
        let router = self
            .router
            .take()
            .expect("Router should be present at startup");
        let _router_task = spawn(router.start());

        #[cfg(feature = "ws")]
        let ws_enabled = !self.config.peer.is_empty();
        #[cfg(feature = "ws_server")]
        let ws_enabled = self.config.ws_server.is_some() || ws_enabled;
        #[cfg(feature = "ws")]
        if ws_enabled {
            tracing::info!("Starting WebSocket Manager");
            self.start_ws_manager();
        }
        #[cfg(feature = "resume_watch")]
        watcher::Watcher::new(self.handle.clone()).start();

        //TODO allow ipc rendezvous filename to be configured by user
        #[cfg(feature = "ipc")]
        if let Some(ipc) = &self.config.ipc
            && ipc.enabled
        {
            let id = self.id;
            let handle = self.handle.clone();
            spawn(async move {
                use crate::peers::IpcManager;

                let manager = IpcManager::new(
                    "anybus.ipc".into(),
                    handle,
                    // self.bc_rx.clone(),
                    id,
                )
                .await;
                manager.start().await
            });
        };
    }

    /// WIP to implement add_bus_stop correctly.  
    pub fn _add_bus_stop<T: BusRider + BusDeserialize>(
        &mut self,
        bus_stop: impl BusStop<T> + 'static + Send,
        id: EndpointId,
    ) {
        let handle = self.handle.clone();
        let service = BusStopService::new(bus_stop, id, handle);
        // service.run()
        tokio::spawn(service.bus_stop_service());
    }

    /// Remove a bus stop
    pub fn remove_bus_stop(&self, _id: BusStopId) -> Result<(), AnyBusHandleError> {
        // TODO: implement stopping the task
        Ok(())
    }
    /// Adds a BusDepot object that will be called by the system when a Rpc request is received
    /// for that EndpointId
    pub fn add_bus_depot<T: BusRiderWithUuid + BusRiderRpc + BusDeserialize>(
        &self,
        bus_depot: impl BusDepot<T> + 'static + Send,
    ) {
        let endpoint = T::ANYBUS_UUID.into();
        self.add_bus_depot_with_endpoint(bus_depot, endpoint);
    }

    /// Adds a BusDepot object that will be called by the system when a Rpc request is received
    /// for that EndpointId
    pub fn add_bus_depot_with_endpoint<T: BusRiderRpc + BusDeserialize>(
        &self,
        bus_depot: impl BusDepot<T> + 'static + Send,
        id: EndpointId,
    ) {
        let handle = self.handle.clone();

        tokio::spawn(async move {
            let handle = handle;
            let mut receiver = match handle.listener().endpoint(id).rpc().register::<T>().await {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("BusStop send failure {}", e);
                    return;
                } // TODO send error message
            };

            let mut bus_depot = bus_depot;

            loop {
                while let Ok(mut request) = receiver.recv().await {
                    let response = bus_depot.on_request(request.payload(), &handle).await;

                    request.reply(response).ok();
                }
            }
        });
    }

    /// Remove a bus depot
    pub fn remove_bus_depot(&self, _id: impl Into<EndpointId>) -> Result<(), AnyBusHandleError> {
        // TODO: implement stopping the task
        Ok(())
    }

    /// Add a remote websocket peer after system startup
    #[cfg(feature = "ws")]
    pub async fn add_websocket_peer(&mut self, url: url::Url, name: String) -> Result<(), String> {
        if self.ws_rpc_client.is_none() {
            self.start_ws_manager();
        }

        let peer_config = crate::anybus::config::WebSocketPeerConfig {
            url: url.try_into()?,
            name,
        };
        if let Some(ws_rpc_client) = &self.ws_rpc_client {
            use crate::peers::ws::AddPeer;

            let res = ws_rpc_client.call(AddPeer { peer_config }).await;

            // let res = ws_command.try_send(peers::ws::WsCommand::AddPeer(url));
            if let Err(e) = res {
                tracing::error!("Failed to send AddPeer command to WebSocketManager: {}", e);
            }
        }
        Ok(())
    }

    /// Remove an existing WebSocket peer
    #[cfg(feature = "ws")]
    pub async fn remove_websocket_peer(&mut self, url: url::Url) -> Result<(), String> {
        if let Some(ws_rpc_client) = &self.ws_rpc_client {
            use crate::peers::ws::RemovePeer;

            return ws_rpc_client
                .call(RemovePeer {
                    url: url.try_into()?,
                })
                .await
                .map_err(|e| {
                    format!(
                        "Failed to send RemovePeer command to WebSocketManager: {}",
                        e
                    )
                })?;
        } else {
            Err("WebSocketManager is not running".to_string())
        }
    }
}

impl Default for AnyBus {
    fn default() -> Self {
        Self::new()
    }
}
