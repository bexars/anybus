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
        use std::time::Duration;

        println!("Ctrl-C received.  Shutting down");
        handle.shutdown(Some(Duration::from_secs(1)));
    }
}

#[cfg(all(not(target_arch = "wasm32"), target_family = "unix"))]
pub(crate) async fn watch_signals(handle: crate::Handle) {
    use tokio::signal::unix::{SignalKind, signal};

    let mut sig_user = signal(SignalKind::user_defined1()).unwrap();
    let mut sig_term = signal(SignalKind::terminate()).unwrap();

    loop {
        use std::time::Duration;

        tokio::select! {
            _ = sig_user.recv() => {
                dbg!(&handle);  // allow this debug since it's diagnotic when it receives a SIG_USR1 (30 on Macos)
            }
            _ = sig_term.recv() => {
                handle.shutdown(Some(Duration::from_millis(100)));
            }
        }
    }
}
