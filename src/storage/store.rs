use super::value::StoreValue;
use customhash::CustomMap;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone)]
pub struct Store {
    pub(crate) data: std::sync::Arc<CustomMap<StoreValue>>,
    pub(crate) connected_clients: std::sync::Arc<AtomicUsize>,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    pub fn new() -> Self {
        Self::with_config(64, 1_000_000)
    }

    pub fn with_config(shards: usize, max_keys: usize) -> Self {
        Self {
            data: std::sync::Arc::new(CustomMap::with_capacity(shards, max_keys)),
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

    pub fn map_shard_count(&self) -> usize {
        self.data.shard_count()
    }
}
