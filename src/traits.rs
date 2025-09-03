use std::any::Any;

use dyn_clone::DynClone;
use erased_serde::Serialize;
use uuid::Uuid;

/// Any message handled by [MsgBus] must have these properties
///
///
pub trait BusRider: Any + DynClone + Serialize + Send + Sync + std::fmt::Debug + 'static {}

dyn_clone::clone_trait_object!(BusRider);

/// Blanket implementation for any type that has the correct traits
///
impl<T: Any + DynClone + Serialize + Send + Sync + std::fmt::Debug + 'static> BusRider for T {}

/// Trait for ease of sending over the bus without the client needing to know the UUID of the default receiver
pub trait BusRiderWithUuid: BusRider {
    /// The default Uuid that will be used if not overridden during registration
    const MSGBUS_UUID: Uuid;
}

/// Trait for denoting the type of a returned RPC response
pub trait BusRiderRpc: BusRider {
    /// The type of the response that will be returned by the RPC call
    type Response: BusRider;
}
