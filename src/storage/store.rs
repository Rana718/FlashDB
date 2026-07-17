use super::value::StoreValue;
use dashmap::DashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone)]
pub struct Store {
    pub(crate) data: DashMap<String, StoreValue>,
    pub(crate) connected_clients: std::sync::Arc<AtomicUsize>,
}

impl Store {
    pub fn new() -> Self {
        // Default DashMap uses 64 shards. For high concurrency workloads,
        // more shards = less contention between threads.
        let shards = (num_cpus::get() * 8).next_power_of_two();
        Self {
            data: DashMap::with_shard_amount(shards),
            connected_clients: std::sync::Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn client_connected(&self) {
        self.connected_clients.fetch_add(1, Ordering::Relaxed);
    }

    pub fn client_disconnected(&self) {
        self.connected_clients.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn connected_clients(&self) -> usize {
        self.connected_clients.load(Ordering::Relaxed)
    }
}

