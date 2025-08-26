use std::any::Any;

use erased_serde::Serialize;
use uuid::Uuid;

/// Any message handled by [MsgBus] must have these properties
///
///
pub trait BusRider: Any + Serialize + Send + Sync + std::fmt::Debug + 'static {}

/// Blanket implementation for any type that has the correct traits
///
impl<T: Any + Serialize + Send + Sync + std::fmt::Debug + 'static> BusRider for T {}

/// Trait for ease of sending over the bus without the client needing to know the UUID of the default receiver
pub trait BusRiderWithUuid: BusRider {
    /// The default Uuid that will be used if not overridden during registration
    const MSGBUS_UUID: Uuid;
}

/// Trait for denoting the type of a returned RPC response
pub trait BusRiderRpc: BusRiderWithUuid {
    /// The type of the response that will be returned by the RPC call
    type Response: BusRider;
}
