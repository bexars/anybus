use std::any::Any;

use dyn_clone::DynClone;

#[cfg(feature = "remote")]
use erased_serde::Serialize;
#[cfg(feature = "serde")]
use serde::de::DeserializeOwned;
use uuid::Uuid;

use async_trait::async_trait;

use crate::errors::AnyBusHandleError;

/// Any message handled by [crate::AnyBus] must have these properties
///
///
#[cfg(feature = "remote")]
pub trait BusRider: Any + DynClone + Serialize + Send + Sync + std::fmt::Debug + 'static {}

/// Any message handled by [crate::AnyBus] must have these properties
///
///
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
/// WIP for removing Serde from Anybus
pub trait BusDeserialize: DeserializeOwned {}

#[cfg(feature = "serde")]
impl<T: Any + DeserializeOwned> BusDeserialize for T {}

#[cfg(not(feature = "serde"))]
impl<T: Any> BusDeserialize for T {}

/// Identifier for a bus stop
pub type BusStopId = Uuid;

/// A ticket representing a message to be sent
#[derive(Debug)]
#[allow(missing_docs)]
pub struct BusTicket {
    pub rider: Box<dyn BusRider>,
    pub dest: Uuid,
}

impl<T: BusRider + BusRiderWithUuid> From<T> for BusTicket {
    fn from(rider: T) -> Self {
        BusTicket {
            rider: Box::new(rider),
            dest: T::ANYBUS_UUID,
        }
    }
}

/// Trait for managed message handlers
pub trait BusStop<T: BusRider> {
    /// Process an incoming message and return tickets for responses
    fn on_message(&self, message: T, handle: &crate::Handle) -> Vec<BusTicket>;
    /// Called when the bus stop is loaded
    fn on_load(&self, _handle: &crate::Handle) -> Result<(), AnyBusHandleError> {
        Ok(())
    }
    /// Called when shutting down
    fn on_shutdown(&self, _handle: &crate::Handle) {}
}

/// Trait for managed RPC services
#[async_trait]
pub trait BusDepot<T: BusRiderRpc>: Send + Sync {
    /// Handle an incoming RPC request
    fn on_request(&self, request: Option<T>, handle: &crate::Handle) -> T::Response;
    /// Called when the depot is loaded and ready
    fn on_load(&self, _handle: &crate::Handle) {}
    /// Called when shutting down
    fn on_shutdown(&self, _handle: &crate::Handle) {}
}
