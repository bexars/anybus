//! Helpers
//! * [ShutdownWithCtrlC] - Wrapper around [BusControlHandle] to intercept Ctrl-C

use std::future::Future;

#[cfg(feature = "dioxus")]
use dioxus::core::Task;
#[cfg(feature = "tokio")]
use tokio::task::JoinHandle;

#[cfg(feature = "tokio")]
use crate::Handle;
// use tokio::{
//     select,
//     sync::oneshot::{Receiver, Sender},
// };

#[cfg(feature = "dioxus")]
/// Convenience function for spawning a task in whichever runtime is being used
pub fn spawn(fut: impl Future<Output = ()> + 'static) -> Task {
    dioxus::prelude::spawn(fut)
}

#[cfg(feature = "tokio")]
/// Convenience function for spawning a task in whichever runtime is being used
#[track_caller]
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(future)
}

// use crate::BusControlHandle;

/// Wrapper struct for handling Ctrl-C input from the terminal.  Receiving Ctrl-C will trigger this to call [BusControlHandle::shutdown()]
///
/// * Unix users should mind the caveat from the Tokio implementation of [tokio::signal::ctrl_c]
// #[cfg(feature = "tokio")]
// pub struct ShutdownWithCtrlC {
//     tx: Sender<()>,
// }

// #[cfg(feature = "tokio")]
// impl ShutdownWithCtrlC {
//     /// Programatically trigger a shutdown of the [crate::MsgBus] system.  This passes through to [crate::BusControlHandle::shutdown()]
//     pub fn shutdown(self) {
//         let _ = self.tx.send(());
//     }
// }

// impl From<MsgBus> for ShutdownWithCtrlC {
//     fn from(msgbus: MsgBus) -> Self {
//         let (tx, rx) = tokio::sync::oneshot::channel::<()>();
//         let this = Self { tx };

//         tokio::spawn(watch_ctrlc(rx, msgbus.handle().clone()));
//         this
//     }
// }

#[cfg(feature = "tokio")]
pub(crate) async fn watch_ctrlc(handle: Handle) {
    if let Ok(_) = tokio::signal::ctrl_c().await {
        println!("Ctrl-C received.  Shutting down");
        handle.shutdown();
    }
}
