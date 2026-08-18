use super::value::StoreValue;
use customhash::CustomMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

pub struct Store {
    pub(crate) data: CustomMap<StoreValue>,
    pub(crate) connected_clients: AtomicUsize,
    pub(crate) ttl_count: AtomicUsize,
    ttl_generation: AtomicU64,
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
            ttl_count: AtomicUsize::new(0),
            ttl_generation: AtomicU64::new(0),
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

    pub fn compact_shard(&self, shard: usize) {
        self.data.compact_shard(shard);
    }

    pub fn map_shard_layout_matches(&self, capacities: &[usize]) -> bool {
        self.data.shard_layout_matches(capacities)
    }

    #[inline]
    pub fn add_ttl(&self) {
        self.ttl_generation.fetch_add(1, Ordering::Release);
        self.ttl_count.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn sub_ttl(&self) {
        let _ = self.ttl_count.fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |count| count.checked_sub(1),
        );
    }

    #[inline]
    pub fn has_ttl_keys(&self) -> bool {
        self.ttl_count.load(Ordering::Relaxed) > 0
    }

    pub fn reset_ttl_count(&self) {
        self.ttl_count.store(0, Ordering::Relaxed);
        self.ttl_generation.fetch_add(1, Ordering::Release);
    }

    #[inline]
    pub fn ttl_generation(&self) -> u64 {
        self.ttl_generation.load(Ordering::Acquire)
    }

    #[inline]
    pub fn finish_ttl_scan(&self, generation: u64, live_ttls: usize) {
        if self.ttl_generation.load(Ordering::Acquire) != generation {
            return;
        }
        let observed = self.ttl_count.load(Ordering::Acquire);
        if self.ttl_generation.load(Ordering::Acquire) != generation {
            return;
        }
        let _ = self.ttl_count.compare_exchange(
            observed,
            live_ttls,
            Ordering::Release,
            Ordering::Relaxed,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::Store;
    use crate::storage::value::StoreValue;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    #[test]
    fn ttl_scan_repairs_overcount_after_overwrite_and_delete() {
        let store = Store::with_config(1, 16);
        let expires_at = Some(Instant::now() + Duration::from_secs(60));

        store.set(
            "key".to_owned(),
            StoreValue::string_with_expiry("one".to_owned(), expires_at),
        );
        store.set(
            "key".to_owned(),
            StoreValue::string_with_expiry("two".to_owned(), expires_at),
        );
        assert_eq!(store.ttl_count.load(Ordering::Relaxed), 2);

        store.cleanup_expired();
        assert_eq!(store.ttl_count.load(Ordering::Relaxed), 1);

        assert!(store.del("key"));
        store.cleanup_expired();
        assert!(!store.has_ttl_keys());
    }

    #[test]
    fn stale_scan_cannot_hide_a_new_ttl() {
        let store = Store::with_config(1, 16);
        let generation = store.ttl_generation();

        store.add_ttl();
        store.finish_ttl_scan(generation, 0);

        assert!(store.has_ttl_keys());
    }

    #[test]
    fn ttl_decrement_saturates_at_zero() {
        let store = Store::with_config(1, 16);
        store.sub_ttl();
        assert!(!store.has_ttl_keys());
    }

    #[test]
    fn getex_updates_ttl_accounting() {
        let store = Store::with_config(1, 16);
        store.set("key".to_owned(), StoreValue::string("value".to_owned()));

        assert_eq!(store.getex_ms("key", u64::MAX), Some("value".to_owned()));
        assert!(store.has_ttl_keys());

        assert_eq!(store.getex_ms("key", 0), Some("value".to_owned()));
        assert!(!store.has_ttl_keys());
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
