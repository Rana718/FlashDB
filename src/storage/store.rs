use super::value::StoreValue;
use customhash::CustomMap;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct Store {
    pub(crate) data: CustomMap<StoreValue>,
    pub(crate) connected_clients: AtomicUsize,
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
            data: CustomMap::with_capacity(shards, max_keys),
            connected_clients: AtomicUsize::new(0),
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

pub fn rss_bytes() -> usize {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<usize>().ok())
        .map(|pages| pages * 4096)
        .unwrap_or(0)
}

pub fn data_memory_bytes() -> usize {
    unsafe extern "C" {
        fn mi_process_info(
            elapsed_msecs: *mut usize,
            user_msecs: *mut usize,
            system_msecs: *mut usize,
            current_rss: *mut usize,
            peak_rss: *mut usize,
            current_commit: *mut usize,
            peak_commit: *mut usize,
            page_faults: *mut usize,
        );
    }
    let mut commit = 0usize;
    unsafe {
        mi_process_info(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut commit,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
    }
    commit
}
