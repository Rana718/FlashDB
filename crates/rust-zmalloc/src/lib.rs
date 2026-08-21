//! Memory control layer using mimalloc as the backing allocator.
//!
//! Provides a `GlobalAlloc` implementation with lightweight memory tracking,
//! RSS reporting, purge support, and raw allocation helpers for EBR-managed
//! data structures.

use std::alloc::{GlobalAlloc, Layout};
use std::sync::atomic::{AtomicUsize, Ordering};

static MIMALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub struct Zmalloc;

unsafe impl GlobalAlloc for Zmalloc {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { MIMALLOC.alloc(layout) }
    }
    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { MIMALLOC.dealloc(ptr, layout) }
    }
    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        unsafe { MIMALLOC.alloc_zeroed(layout) }
    }
    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        unsafe { MIMALLOC.realloc(ptr, layout, new_size) }
    }
}

/// Approximate memory usage. Updated lazily via `refresh_used_memory()`.
static USED_MEMORY: AtomicUsize = AtomicUsize::new(0);

/// Returns total bytes currently allocated (cached value, updated by `refresh_used_memory`).
#[inline]
pub fn used_memory() -> usize {
    USED_MEMORY.load(Ordering::Relaxed)
}

/// Refresh the used_memory counter by reading RSS.
/// Called periodically from the maintenance thread, not on every alloc/dealloc.
pub fn refresh_used_memory() {
    let rss = rss_bytes_inner();
    USED_MEMORY.store(rss, Ordering::Relaxed);
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Stats {
    pub allocated: usize,
    pub active: usize,
    pub resident: usize,
    pub retained: usize,
    pub muzzy: usize,
}

pub fn stats() -> Stats {
    let rss = rss_bytes_inner();
    Stats {
        allocated: rss,
        active: rss,
        resident: rss,
        retained: 0,
        muzzy: 0,
    }
}

/// Force mimalloc to collect and return unused pages to the OS.
pub fn purge() {
    unsafe extern "C" {
        fn mi_collect(force: bool);
    }
    unsafe { mi_collect(true) };
}

#[inline]
pub fn refresh_epoch() {}

pub fn fragmentation_ratio() -> f64 {
    1.0
}

/// Raw allocation helper for EBR-managed objects.
#[inline]
pub unsafe fn alloc_raw(layout: Layout) -> *mut u8 {
    unsafe { MIMALLOC.alloc(layout) }
}

#[inline]
pub unsafe fn dealloc_raw(ptr: *mut u8, layout: Layout) {
    if !ptr.is_null() {
        unsafe { MIMALLOC.dealloc(ptr, layout) }
    }
}

#[inline]
pub unsafe fn alloc_raw_no_tcache(layout: Layout) -> *mut u8 {
    unsafe { MIMALLOC.alloc(layout) }
}

#[inline]
pub unsafe fn dealloc_raw_no_tcache(ptr: *mut u8, layout: Layout) {
    if !ptr.is_null() {
        unsafe { MIMALLOC.dealloc(ptr, layout) }
    }
}

fn rss_bytes_inner() -> usize {
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
            .unwrap_or(0)
    }
    #[cfg(not(target_os = "linux"))]
    {
        0
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
}
