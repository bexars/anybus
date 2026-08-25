use crate::anybus::AnyBus;

/// AnyBusBuilder is a builder pattern for constructing an AnyBus instance with options
#[derive(Debug, Default, Clone)]
pub struct AnyBusBuilder {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) enable_ctrlc_shutdown: bool,
    #[cfg(feature = "ipc")]
    pub(crate) enable_ipc: bool,
    #[cfg(feature = "ws_server")]
    pub(crate) ws_listener_options: Option<crate::peers::WsListenerOptions>,
    #[cfg(feature = "ws")]
    pub(crate) ws_remote_options: Vec<crate::peers::WsRemoteOptions>,
}
impl AnyBusBuilder {
    /// Creates a new AnyBusBuilder with default options
    pub fn new() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            enable_ctrlc_shutdown: false,
            #[cfg(feature = "ipc")]
            enable_ipc: false,
            #[cfg(feature = "ws_server")]
            ws_listener_options: None,
            #[cfg(feature = "ws")]
            ws_remote_options: Vec::new(),
        }
    }

    /// Enables or disables the Ctrl-C shutdown feature.  Default is disabled.
    ///
    /// If enabled, when the user presses Ctrl-C in the terminal, the AnyBus system will be shutdown cleanly
    ///
    #[cfg(not(target_arch = "wasm32"))]
    pub fn enable_ctrlc_shutdown(mut self, enable: bool) -> Self {
        self.enable_ctrlc_shutdown = enable;
        self
    }

    /// Enables or disables the IPC peer discovery and messaging feature.  Default is enabled.
    ///
    /// If disabled, this AnyBus instance will not be able to discover or communicate with other local AnyBus instances using IPC
    ///
    #[cfg(feature = "ipc")]
    pub fn enable_ipc(mut self, enable: bool) -> Self {
        self.enable_ipc = enable;
        self
    }

    /// Sets the WebSocket listener options.  If set, a WebSocket listener will be started with these options.
    ///
    /// If not set, no WebSocket listener will be started.
    ///
    #[cfg(feature = "ws_server")]
    pub fn ws_listener(mut self, options: crate::peers::WsListenerOptions) -> Self {
        self.ws_listener_options = Some(options);
        self
    }

    /// Adds a remote WebSocket peer to connect to.  Can be called multiple times to add multiple remote peers.
    #[cfg(feature = "ws")]
    pub fn ws_remote(mut self, options: crate::peers::WsRemoteOptions) -> Self {
        self.ws_remote_options.push(options);
        self
    }

    /// Builds an AnyBus instance with the specified options.  Returns the AnyBus instance.
    ///
    pub fn init(&self) -> AnyBus {
        let anybus = AnyBus::init(self.clone());

        anybus
    }

    /// Initializes and starts an AnyBus instance with the specified options.  Returns the AnyBus instance.
    ///
    pub fn run(&self) -> AnyBus {
        let mut anybus = self.init();
        anybus.run();
        anybus
    }
}
