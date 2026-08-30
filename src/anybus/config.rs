//! Module to interact with a standard anybus TOML config file
//!

use std::{collections::HashMap, fmt::Display, net::IpAddr};

use crate::AnyBusBuilder;
use serde::{Deserialize, Deserializer};
#[cfg(feature = "ws")]
use url::Url;

/// Structure for configuring a new AnyBus instance
#[derive(Deserialize, Debug, Clone)]
pub struct AnyBusConfig {
    #[serde(default)]
    #[cfg(feature = "ipc")]
    pub(crate) ipc: Option<IpcConfig>,
    #[serde(default)]
    pub(crate) peers: HashMap<String, PeerType>,
    #[serde(default)]
    #[cfg(feature = "ws_server")]
    pub(crate) ws_server: Option<WebSocketServerConfig>,
    #[serde(default)]
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) enable_ctrlc_shutdown: bool,
}

impl AnyBusConfig {
    #[cfg(feature = "toml")]
    /// Loads an AnyBusConfig from a .toml file
    /// See examples/relay/dummy for an example file
    pub fn load_config(
        path: &std::path::PathBuf,
    ) -> Result<AnyBusConfig, Box<dyn std::error::Error>> {
        let config_str = std::fs::read_to_string(path)?;
        let config: AnyBusConfig = toml::from_str(&config_str)?;
        Ok(config)
    }
}

/// Temporary crutch while phasing out the old AnyBusBuilder
impl From<AnyBusBuilder> for AnyBusConfig {
    fn from(builder: AnyBusBuilder) -> Self {
        let mut peers = HashMap::new();
        #[cfg(feature = "ws")]
        for (index, peer) in builder.ws_remote_options.iter().enumerate() {
            peers.insert(
                format!("ws_remote_{}", index),
                PeerType::WebSocket {
                    url: WsUrl(peer.url.clone()),
                },
            );
        }
        Self {
            enable_ctrlc_shutdown: builder.enable_ctrlc_shutdown,
            #[cfg(feature = "ipc")]
            ipc: Some(IpcConfig {
                enabled: builder.enable_ipc,
            }),
            peers,

            #[cfg(feature = "ws_server")]
            ws_server: builder
                .ws_listener_options
                .map(|options| WebSocketServerConfig {
                    address: options.addr,
                    port: options.port,
                    cert_path: options.cert_path,
                    key_path: options.key_path,
                    enable_tls: options.use_tls,
                }),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
pub(crate) struct IpcConfig {
    #[serde(default)]
    pub(crate) enabled: bool,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub(crate) enum PeerType {
    #[serde(rename = "ws")]
    #[cfg(feature = "ws")]
    WebSocket { url: WsUrl },
}

#[cfg(feature = "ws")]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WebSocketPeerConfig {
    pub(crate) url: WsUrl,
    pub(crate) name: String,
}

impl Display for WebSocketPeerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "WebSocket {{ name: {}, url: {} }}",
            self.name, self.url.0
        )
    }
}

struct PeerMap(HashMap<String, PeerType>);

impl From<PeerMap> for Vec<WebSocketPeerConfig> {
    fn from(peer_map: PeerMap) -> Self {
        peer_map
            .0
            .into_iter()
            .filter_map(|(name, peer)| {
                #[allow(irrefutable_let_patterns)] // remove when there's a 2nd peer type
                if let PeerType::WebSocket { url } = peer {
                    Some(WebSocketPeerConfig { url, name })
                } else {
                    None
                }
            })
            .collect()
    }
}

// #[derive(Deserialize, Debug, Clone)]
// pub(crate) struct WebSocketConfig {
//     pub(crate) url: String,
// }

#[derive(Deserialize, Debug, Clone)]
#[cfg(feature = "ws_server")]
pub(crate) struct WebSocketServerConfig {
    #[serde(default = "default_ipv6_any")]
    pub(crate) address: IpAddr,
    #[serde(default = "default_port")]
    pub(crate) port: u16,
    pub(crate) cert_path: Option<String>,
    pub(crate) key_path: Option<String>,
    pub(crate) enable_tls: bool,
}

fn default_ipv6_any() -> IpAddr {
    "::".parse().unwrap()
}

fn default_port() -> u16 {
    9798
}

impl AnyBusConfig {
    #[cfg(feature = "ws")]
    /// Add websocket with the given url and a name for internal use in logging and displays
    pub fn add_ws_peer(mut self, name: String, url: String) {
        {
            let url = parse_ws_url(&url).expect("Invalid websocket URL");
            self.peers
                .insert(name, PeerType::WebSocket { url: WsUrl(url) });
        }
    }

    #[cfg(feature = "ws_server")]
    /// Set the websocket server configuration
    pub fn set_ws_server(
        mut self,
        address: IpAddr,
        port: u16,
        cert: Option<String>,
        key: Option<String>,
        enable_tls: bool,
    ) {
        self.ws_server = Some(WebSocketServerConfig {
            address,
            port,
            cert_path: cert,
            key_path: key,
            enable_tls,
        });
    }

    #[cfg(feature = "ipc")]
    /// Enable or disable IPC peer discovery and messaging
    pub fn set_ipc_enabled(mut self, enabled: bool) {
        self.ipc = Some(IpcConfig { enabled });
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Enable intercepting Ctrl-c to trigger a graceful shutdown of Anybus.  This will send a Shutdown message
    /// to all receivers, Handle, and AnyBusStatusMsg and stop accepting any new packets
    pub fn set_ctrlc_shutdown(mut self, enabled: bool) {
        self.enable_ctrlc_shutdown = enabled;
    }
}

fn parse_ws_url(s: &str) -> Result<Url, String> {
    let url = Url::parse(s).map_err(|e| e.to_string())?;
    match url.scheme() {
        "ws" | "wss" => Ok(url),
        other => Err(format!("expected ws:// or wss://, got `{other}`")),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WsUrl(Url);

impl From<WsUrl> for Url {
    fn from(ws_url: WsUrl) -> Self {
        ws_url.0
    }
}

impl TryFrom<Url> for WsUrl {
    type Error = String;

    fn try_from(value: Url) -> Result<Self, Self::Error> {
        match value.scheme() {
            "ws" | "wss" => Ok(WsUrl(value)),
            other => Err(format!("expected ws:// or wss://, got `{other}`")),
        }
    }
}

impl Display for WsUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl<'de> Deserialize<'de> for WsUrl {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let url = Url::deserialize(deserializer)?;
        match url.scheme() {
            "ws" | "wss" => Ok(WsUrl(url)),
            other => Err(serde::de::Error::custom(format!(
                "expected ws:// or wss://, got `{other}`"
            ))),
        }
    }
}
