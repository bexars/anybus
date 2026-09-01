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
    atomic::{AtomicUsize, Ordering},
};

#[derive(Debug, Clone)]
pub(crate) struct Counter {
    current: u16,
}

impl Counter {
    pub(crate) fn new() -> Self {
        Self { current: 0 }
    }
}

impl Iterator for Counter {
    type Item = u16;

    fn next(&mut self) -> Option<Self::Item> {
        let val = self.current;
        self.current += 1;
        Some(val)
    }
}

#[derive(Clone)]
pub(crate) struct SharedCounter {
    // Arc allows multiple tasks to own a reference to this same memory
    current: Arc<AtomicUsize>,
}

impl SharedCounter {
    fn new(start: usize) -> Self {
        SharedCounter {
            current: Arc::new(AtomicUsize::new(start)),
        }
    }

    fn next_value(&self) -> usize {
        // Fetch the current value and increment it by 1 atomically
        self.current.fetch_add(1, Ordering::SeqCst)
    }
}
