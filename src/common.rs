#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Used to control how the route is advertised
#[derive(Debug, Copy, Clone, PartialEq, Default, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[allow(dead_code)]
pub enum Realm {
    /// Only within the current process
    Process,
    /// Within the current userspace instance (multiple processes on same machine)
    #[cfg(feature = "remote")]
    Userspace,
    /// Within the local network (LAN)
    #[cfg(feature = "remote")]
    LocalNet,
    /// Globally routable (Websocket)
    #[default]
    Global,
    // BroadcastProxy(EndpointId),
}
