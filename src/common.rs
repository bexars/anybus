// use uuid::Uuid;

// pub fn random_uuid() -> Uuid {
//     // use rand::prelude::*;
//     let random_bytes = rand::random();

//     // rng().random();

//     let uuid = uuid::Builder::from_random_bytes(random_bytes).into_uuid();

//     uuid
// }

use std::sync::{
    Arc,
    atomic::{AtomicU16, Ordering},
};

#[derive(Clone)]
pub(crate) struct SharedCounter {
    // Arc allows multiple tasks to own a reference to this same memory
    current: Arc<AtomicU16>,
}

impl std::fmt::Debug for SharedCounter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SharedCounter {{ current: {} }}",
            self.current.load(Ordering::SeqCst)
        )
    }
}

impl SharedCounter {
    pub(crate) fn new(start: u16) -> Self {
        SharedCounter {
            current: Arc::new(AtomicU16::new(start)),
        }
    }

    pub(crate) fn next(&self) -> u16 {
        // Fetch the current value and increment it by 1 atomically
        self.current.fetch_add(1, Ordering::SeqCst)
    }
}
