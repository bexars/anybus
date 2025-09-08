#[cfg(feature = "dioxus")]
use std::future::Future;

#[cfg(feature = "dioxus")]
use dioxus::core::Task;
#[cfg(feature = "tokio")]
use tokio::task::JoinHandle;

#[cfg(feature = "tokio")]
use crate::Handle;

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

/// Wrapper struct for handling Ctrl-C input from the terminal.  Receiving Ctrl-C will trigger this to call [BusControlHandle::shutdown()]
///
/// * Unix users should mind the caveat from the Tokio implementation of [tokio::signal::ctrl_c]

#[cfg(feature = "tokio")]
pub(crate) async fn watch_ctrlc(handle: Handle) {
    if let Ok(_) = tokio::signal::ctrl_c().await {
        println!("Ctrl-C received.  Shutting down");
        handle.shutdown();
    }
}
