//! Counts of things that are open at a time, with a limit for each key.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex, PoisonError};

/// Counts open things (connections, waiting calls) for each key.
#[derive(Debug)]
pub(crate) struct Gauge<K> {
    open: Mutex<HashMap<K, u32>>,
}

/// One open thing. Dropping it gives the place back.
#[derive(Debug)]
pub(crate) struct Slot<K: Eq + Hash + Clone> {
    gauge: Arc<Gauge<K>>,
    key: K,
}

impl<K: Eq + Hash + Clone> Gauge<K> {
    pub(crate) fn new() -> Arc<Gauge<K>> {
        Arc::new(Gauge { open: Mutex::new(HashMap::new()) })
    }

    /// Takes one place of `key`. Returns `None` when `key` has `max`
    /// places already. A `max` of 0 means no limit.
    pub(crate) fn acquire(self: &Arc<Self>, key: K, max: u32) -> Option<Slot<K>> {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        let count = open.entry(key.clone()).or_insert(0);
        if max > 0 && *count >= max {
            return None;
        }
        *count += 1;
        Some(Slot { gauge: Arc::clone(self), key })
    }

    /// Returns the number of places that are taken, over all keys.
    pub(crate) fn total(&self) -> u64 {
        let open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        open.values().map(|&count| u64::from(count)).sum()
    }
}

impl<K: Eq + Hash + Clone> Drop for Slot<K> {
    fn drop(&mut self) {
        let mut open = self.gauge.open.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = open.get_mut(&self.key) {
            *count -= 1;
            if *count == 0 {
                open.remove(&self.key);
            }
        }
    }
}
