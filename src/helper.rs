//! Helper functions:  Currently a platform agnost spawn() for creating tasks and the ctrl-c shutdown helper

// use tokio_with_wasm::alias as tokio;
#[cfg(feature = "dioxus")]
use dioxus::dioxus_core::Task;

// #[cfg(not(feature = "dioxus"))]
// use crate::Handle;

#[cfg(feature = "dioxus")]
/// Convenience function for spawning a task in whichever runtime is being used
pub fn spawn(fut: impl Future<Output = ()> + 'static) -> Task {
    dioxus::prelude::spawn(fut)
}

#[cfg(all(target_arch = "wasm32", not(feature = "dioxus")))]
/// Convenience function for spawning a task in whichever runtime is being used
#[track_caller]
pub fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(future)
}
#[cfg(all(not(target_arch = "wasm32"), not(feature = "dioxus")))]
// #[cfg(not(target_arch = "wasm32"))]
/// Convenience function for spawning a task in whichever runtime is being used
#[track_caller]
pub fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(future)
}

/// Wrapper struct for handling Ctrl-C input from the terminal.  Receiving Ctrl-C will trigger the internal shutdown procedure
///
/// * Unix users should mind the caveat from the Tokio implementation of [tokio::signal::ctrl_c]
#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn watch_ctrlc(handle: crate::Handle) {
    if let Ok(_) = tokio::signal::ctrl_c().await {
        println!("Ctrl-C received.  Shutting down");
        handle.shutdown();
    }
}
