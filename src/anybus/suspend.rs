use crate::tokio;
use crate::{AnyBusStatusMsg, Handle, spawn};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;

const DEBOUNCE: Duration = Duration::from_secs(3);
const STALL_TICK: Duration = Duration::from_secs(1);
const STALL_GAP: Duration = Duration::from_secs(10);

pub(crate) struct LifecycleDebounce {
    last: Mutex<Option<(AnyBusStatusMsg, Instant)>>,
}

impl Default for LifecycleDebounce {
    fn default() -> Self {
        Self {
            last: Mutex::new(None),
        }
    }
}

impl LifecycleDebounce {
    fn allow(&self, msg: AnyBusStatusMsg) -> bool {
        let now = Instant::now();
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((prev, at)) = *last
            && prev == msg
            && now.saturating_duration_since(at) < DEBOUNCE
        {
            return false;
        }
        *last = Some((msg, now));
        true
    }
}

pub(crate) fn emit(handle: &Handle, debounce: &LifecycleDebounce, msg: AnyBusStatusMsg) {
    if !matches!(msg, AnyBusStatusMsg::Suspending | AnyBusStatusMsg::Resuming) {
        return;
    }
    if !debounce.allow(msg) {
        tracing::debug!("Debounced {msg:?}");
        return;
    }
    tracing::info!("AnyBus {msg:?}");
    if let Err(e) = handle.send(msg) {
        tracing::debug!("Failed to send {msg:?}: {e}");
    }
}

pub(crate) fn start(handle: Handle) {
    let debounce = Arc::new(LifecycleDebounce::default());
    spawn(stall_loop(handle.clone(), debounce.clone()));
    #[cfg(feature = "resume_watch")]
    super::watcher::Watcher::new(handle.clone(), debounce.clone()).start();
    #[cfg(target_arch = "wasm32")]
    spawn(wasm_visibility(handle, debounce));
}

fn wall_now_ms() -> u128 {
    #[cfg(target_arch = "wasm32")]
    {
        // std::time::SystemTime panics in the browser ("time not implemented").
        js_sys::Date::now() as u128
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    }
}

async fn stall_loop(handle: Handle, debounce: Arc<LifecycleDebounce>) {
    // Wall clock, not Instant: Instant is monotonic uptime and often pauses
    // in suspend / frozen WASM tabs, so the gap after wake would look like 1s.
    let mut last = wall_now_ms();
    loop {
        tokio::time::sleep(STALL_TICK).await;
        let now = wall_now_ms();
        let gap = Duration::from_millis(now.saturating_sub(last) as u64);
        last = now;
        if gap >= STALL_GAP {
            emit(&handle, &debounce, AnyBusStatusMsg::Resuming);
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn wasm_visibility(handle: Handle, debounce: Arc<LifecycleDebounce>) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use web_sys::VisibilityState;

    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let doc = document.clone();
    let callback = Closure::wrap(Box::new(move || {
        let msg = if doc.visibility_state() == VisibilityState::Hidden {
            AnyBusStatusMsg::Suspending
        } else {
            AnyBusStatusMsg::Resuming
        };
        tx.send(msg).ok();
    }) as Box<dyn FnMut()>);
    if document
        .add_event_listener_with_callback("visibilitychange", callback.as_ref().unchecked_ref())
        .is_err()
    {
        return;
    }
    callback.forget();
    while let Some(msg) = rx.recv().await {
        emit(&handle, &debounce, msg);
    }
}
