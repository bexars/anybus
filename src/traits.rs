use std::any::Any;

use dyn_clone::DynClone;
#[cfg(feature = "serde")]
use erased_serde::Serialize;
#[cfg(feature = "serde")]
use serde::de::DeserializeOwned;
use uuid::Uuid;

/// Any message handled by [crate::AnyBus] must have these properties
///
///
#[cfg(feature = "remote")]
pub trait BusRider: Any + DynClone + Serialize + Send + Sync + std::fmt::Debug + 'static {}
#[cfg(not(feature = "remote"))]
pub trait BusRider: Any + DynClone + Send + Sync + std::fmt::Debug + 'static {}

dyn_clone::clone_trait_object!(BusRider);

/// Blanket implementation for any type that has the correct traits
///

#[cfg(feature = "remote")]
impl<T: Any + DynClone + Serialize + Send + Sync + std::fmt::Debug + 'static> BusRider for T {}
#[cfg(not(feature = "remote"))]
impl<T: Any + DynClone + Send + Sync + std::fmt::Debug + 'static> BusRider for T {}

/// Trait for ease of sending over the bus without the client needing to know the UUID of the default receiver
pub trait BusRiderWithUuid: BusRider {
    /// The default Uuid that will be used if not overridden during registration
    const ANYBUS_UUID: Uuid;
}

/// Trait for denoting the type of a returned RPC response
pub trait BusRiderRpc: BusRider {
    /// The type of the response that will be returned by the RPC call
    type Response: BusRider;
}

/// Helper trait for
#[cfg(not(feature = "serde"))]
pub trait BusDeserialize {}
#[cfg(feature = "serde")]
pub trait BusDeserialize: DeserializeOwned {}

#[cfg(feature = "serde")]
impl<T: Any + DeserializeOwned> BusDeserialize for T {}

#[cfg(not(feature = "serde"))]
impl<T: Any> BusDeserialize for T {}
