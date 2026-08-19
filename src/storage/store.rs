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
    rss_bytes()
}

/// Bytes currently allocated by jemalloc (excludes untouched/retained RSS).
pub fn allocated_bytes() -> usize {
    #[cfg(not(target_env = "msvc"))]
    unsafe {
        refresh_allocator_epoch();
        let name = b"stats.allocated\0";
        let mut value: usize = 0;
        let mut size = std::mem::size_of::<usize>();
        if jemalloc_sys::mallctl(
            name.as_ptr().cast(),
            (&mut value as *mut usize).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        ) == 0 {
            return value;
        }
    }
    rss_bytes()
}

pub fn peak_rss_bytes() -> usize {
    proc_status_kb("VmHWM:").saturating_mul(1024)
}

/// Ask jemalloc to purge dirty/muzzy pages immediately after an explicit
/// destructive command such as FLUSHALL. This is demand-driven and does not
/// add a polling thread or affect normal allocation performance.
pub fn purge_allocator() {
    #[cfg(not(target_env = "msvc"))]
    unsafe {
        // jemalloc caches statistics and decay state behind the epoch mallctl.
        // Refresh it before purging so pages freed by other worker threads are
        // visible to the purge operation, including musl builds where
        // background_thread is unavailable.
        refresh_allocator_epoch();
        let mut arenas: libc::c_uint = 0;
        let mut size = std::mem::size_of::<libc::c_uint>();
        let narenas = b"arenas.narenas\0";
        if jemalloc_sys::mallctl(
            narenas.as_ptr().cast(),
            (&mut arenas as *mut libc::c_uint).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0 {
            return;
        }
        for arena in 0..arenas {
            let name = format!("arena.{arena}.purge\0");
            let _ = jemalloc_sys::mallctl(
                name.as_ptr().cast(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            );
        }
    }
}

#[cfg(not(target_env = "msvc"))]
unsafe fn refresh_allocator_epoch() {
    let name = b"epoch\0";
    let mut epoch: usize = 1;
    let mut size = std::mem::size_of::<usize>();
    let _ = unsafe {
        jemalloc_sys::mallctl(
            name.as_ptr().cast(),
            (&mut epoch as *mut usize).cast(),
            &mut size,
            (&mut epoch as *mut usize).cast(),
            size,
        )
    };
}

pub fn cgroup_memory_bytes() -> usize {
    ["/sys/fs/cgroup/memory.current", "/sys/fs/cgroup/memory/memory.usage_in_bytes"]
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
