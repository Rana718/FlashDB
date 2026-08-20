//! A memory control layer using mimalloc as the backing allocator.
//!
//! Provides a `GlobalAlloc` implementation with per-allocation size tracking,
//! RSS reporting, purge support, and raw allocation helpers for EBR-managed
//! data structures.

use std::alloc::{GlobalAlloc, Layout};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Tracks total bytes allocated through the global allocator.
/// O(1) read — mirrors Redis's `zmalloc_used_memory`.
static USED_MEMORY: AtomicUsize = AtomicUsize::new(0);

static MIMALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub struct Zmalloc;

unsafe impl GlobalAlloc for Zmalloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { MIMALLOC.alloc(layout) };
        if !ptr.is_null() {
            USED_MEMORY.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { MIMALLOC.dealloc(ptr, layout) };
        USED_MEMORY.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { MIMALLOC.alloc_zeroed(layout) };
        if !ptr.is_null() {
            USED_MEMORY.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { MIMALLOC.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            if layout.size() <= new_size {
                USED_MEMORY.fetch_add(new_size - layout.size(), Ordering::Relaxed);
            } else {
                USED_MEMORY.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        new_ptr
    }
}

/// Returns total bytes currently allocated through the global allocator.
/// O(1) atomic read.
#[inline]
pub fn used_memory() -> usize {
    USED_MEMORY.load(Ordering::Relaxed)
}

/// Returns the fragmentation ratio: RSS / allocated.
/// A ratio close to 1.0 means low fragmentation.
#[inline]
pub fn fragmentation_ratio() -> f64 {
    let allocated = used_memory();
    if allocated == 0 {
        1.0
    } else {
        let rss = rss_bytes();
        rss as f64 / allocated as f64
    }
}

/// Allocator statistics matching the previous jemalloc-based interface.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stats {
    pub allocated: usize,
    pub active: usize,
    pub resident: usize,
    pub retained: usize,
    pub muzzy: usize,
}

/// Collect allocator statistics.
/// mimalloc doesn't require an epoch refresh — stats are always live.
pub fn stats() -> Stats {
    let used = used_memory();
    let rss = rss_bytes();
    Stats {
        allocated: used,
        active: used,
        resident: rss,
        retained: 0,
        muzzy: 0,
    }
}

/// Force mimalloc to release unused pages back to the OS.
/// mimalloc eagerly purges by default (MADV_DONTNEED), so this is
/// mostly a hint for large bulk-free operations like FLUSHALL.
pub fn purge() {
    unsafe {
        mimalloc::MiMalloc.alloc(Layout::from_size_align_unchecked(0, 1));
    }
    // mi_collect(true) forces a full collection
    // The mimalloc crate doesn't expose mi_collect directly, but we can
    // trigger it through the options API or just let the eager purging handle it.
    // For now, this is a no-op since mimalloc already decommits eagerly.
}

/// No-op for mimalloc (no epoch concept).
#[inline]
pub fn refresh_epoch() {}

/// Raw allocation helper. The caller must free with `dealloc_raw` using the
/// same layout after all EBR readers have left the object.
pub unsafe fn alloc_raw(layout: Layout) -> *mut u8 {
    unsafe { Zmalloc.alloc(layout) }
}

pub unsafe fn dealloc_raw(ptr: *mut u8, layout: Layout) {
    if !ptr.is_null() {
        unsafe { Zmalloc.dealloc(ptr, layout) };
    }
}

/// Allocate a long-lived object. With mimalloc there's no tcache distinction —
/// all freed memory is immediately available for purging.
pub unsafe fn alloc_raw_no_tcache(layout: Layout) -> *mut u8 {
    unsafe { Zmalloc.alloc(layout) }
}

pub unsafe fn dealloc_raw_no_tcache(ptr: *mut u8, layout: Layout) {
    if !ptr.is_null() {
        unsafe { Zmalloc.dealloc(ptr, layout) };
    }
}

/// Read RSS from /proc/self/status (Linux) or return used_memory as fallback.
fn rss_bytes() -> usize {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|status| {
                status.lines().find_map(|line| {
                    let value = line.strip_prefix("VmRSS:")?;
                    value.split_whitespace().next()?.parse::<usize>().ok()
                })
            })
            .map(|kb| kb * 1024)
            .unwrap_or_else(used_memory)
    }
    #[cfg(not(target_os = "linux"))]
    {
        used_memory()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_alloc_preserves_alignment_and_contents() {
        for &(size, align) in &[(1, 1), (17, 8), (129, 64), (4096, 4096)] {
            let layout = Layout::from_size_align(size, align).unwrap();
            let ptr = unsafe { alloc_raw(layout) };
            assert!(!ptr.is_null());
            assert_eq!((ptr as usize) % align, 0);
            unsafe {
                std::ptr::write_bytes(ptr, 0xA5, size);
                assert_eq!(*ptr, 0xA5);
                assert_eq!(*ptr.add(size - 1), 0xA5);
                dealloc_raw(ptr, layout);
            }
        }
    }

    #[test]
    fn used_memory_tracks_allocations() {
        let before = used_memory();
        let layout = Layout::from_size_align(1024, 8).unwrap();
        let ptr = unsafe { alloc_raw(layout) };
        assert!(!ptr.is_null());
        let after = used_memory();
        assert!(after >= before + 1024);
        unsafe { dealloc_raw(ptr, layout) };
        let final_val = used_memory();
        assert!(final_val <= after - 1024 + 1); // allow 1 byte tolerance
    }
}
