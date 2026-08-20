use super::value::StoreValue;
use customhash::CustomMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

pub struct Store {
    pub(crate) data: CustomMap<StoreValue>,
    pub(crate) connected_clients: AtomicUsize,
    pub(crate) ttl_count: AtomicUsize,
    ttl_generation: AtomicU64,
    pub(crate) counter_lock: Mutex<()>,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    pub fn new() -> Self {
        Self::with_config(1, 1_000)
    }

    pub fn with_config(shards: usize, max_keys: usize) -> Self {
        Self {
            data: CustomMap::with_capacity(shards, max_keys),
            connected_clients: AtomicUsize::new(0),
            ttl_count: AtomicUsize::new(0),
            ttl_generation: AtomicU64::new(0),
            counter_lock: Mutex::new(()),
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

    pub fn map_shard_slot_count(&self, shard: usize) -> usize {
        self.data.shard_slot_count(shard)
    }

    pub fn compact_shard(&self, shard: usize) {
        self.data.compact_shard(shard);
    }

    /// Reclaims oversized shard tables after workload shrinkage. The map's
    /// compaction gate serializes each shard and readers remain EBR-safe.
    pub fn compact_underutilized(&self) {
        for shard in 0..self.map_shard_count() {
            self.data.compact_shard(shard);
        }
        customhash::force_collect_quiescent();
    }

    pub fn defragment_values(&self, budget: usize) -> usize {
        let rebuilt = self
            .data
            .defragment_values(budget, |value| value.compact_allocations());
        customhash::force_collect_quiescent();
        rebuilt
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
        let _ = self
            .ttl_count
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                count.checked_sub(1)
            });
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
    // mimalloc eagerly decommits pages (MADV_DONTNEED) so VmRSS accurately
    // reflects actual memory usage without transient inflation.
    proc_status_kb("VmRSS:").saturating_mul(1024)
}

pub fn data_memory_bytes() -> usize {
    rss_bytes()
}

/// Bytes currently allocated by the allocator.
pub fn allocated_bytes() -> usize {
    rust_zmalloc::used_memory()
}

pub fn peak_rss_bytes() -> usize {
    proc_status_kb("VmHWM:").saturating_mul(1024)
}

/// Ask the allocator to release unused pages back to the OS.
/// mimalloc eagerly decommits by default, so this is mostly a hint
/// after bulk-free operations like FLUSHALL.
pub fn purge_allocator() {
    rust_zmalloc::purge();
}

/// Purge only when fragmentation is material.
pub fn purge_allocator_if_fragmented() {
    let used = rust_zmalloc::used_memory();
    let rss = rss_bytes();
    // If RSS is more than 20% above used memory, trigger a purge
    if rss > used.saturating_add(used / 5) && rss.saturating_sub(used) >= 10 * 1024 * 1024 {
        rust_zmalloc::purge();
    }
}

pub fn cgroup_memory_bytes() -> usize {
    [
        "/sys/fs/cgroup/memory.current",
        "/sys/fs/cgroup/memory/memory.usage_in_bytes",
    ]
    .iter()
    .find_map(|path| std::fs::read_to_string(path).ok()?.trim().parse().ok())
    .unwrap_or(0)
}

fn proc_status_kb(name: &str) -> usize {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                let value = line.strip_prefix(name)?;
                value.split_whitespace().next()?.parse().ok()
            })
        })
        .unwrap_or(0)
}
