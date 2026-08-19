//! A reusable Rust equivalent of Redis's `zmalloc` control layer.
//!
//! The allocator remains Rust's `GlobalAlloc` interface, while exposing
//! jemalloc statistics, epoch refresh, purge, and explicit raw allocation
//! helpers for future ownership-aware data structures.

use std::alloc::{GlobalAlloc, Layout};
use std::ptr::null_mut;

pub struct Zmalloc;

unsafe impl GlobalAlloc for Zmalloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { jemallocator::Jemalloc.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { jemallocator::Jemalloc.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        unsafe { jemallocator::Jemalloc.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        unsafe { jemallocator::Jemalloc.realloc(ptr, layout, size) }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Stats {
    pub allocated: usize,
    pub active: usize,
    pub resident: usize,
    pub retained: usize,
    pub muzzy: usize,
}

#[inline]
pub fn stats() -> Stats {
    refresh_epoch();
    let page = read_usize(b"arenas.page\0");
    let arenas = read_u32(b"arenas.narenas\0");
    let mut muzzy_pages = 0usize;
    for arena in 0..arenas {
        muzzy_pages = muzzy_pages.saturating_add(read_usize(
            format!("stats.arenas.{arena}.pmuzzy\0").as_bytes(),
        ));
    }
    Stats {
        allocated: read_usize(b"stats.allocated\0"),
        active: read_usize(b"stats.active\0"),
        resident: read_usize(b"stats.resident\0"),
        retained: read_usize(b"stats.retained\0"),
        muzzy: muzzy_pages.saturating_mul(page),
    }
}

pub fn purge() {
    refresh_epoch();
    let arenas = read_u32(b"arenas.narenas\0");
    for arena in 0..arenas {
        let name = format!("arena.{arena}.purge\0");
        unsafe {
            let _ =
                jemalloc_sys::mallctl(name.as_ptr().cast(), null_mut(), null_mut(), null_mut(), 0);
        }
    }
}

#[inline]
pub fn refresh_epoch() {
    let name = b"epoch\0";
    let mut epoch: usize = 1;
    let mut size = std::mem::size_of::<usize>();
    unsafe {
        let _ = jemalloc_sys::mallctl(
            name.as_ptr().cast(),
            (&mut epoch as *mut usize).cast(),
            &mut size,
            (&mut epoch as *mut usize).cast(),
            size,
        );
    }
}

#[inline]
fn read_usize(name: &[u8]) -> usize {
    let mut value = 0usize;
    let mut size = std::mem::size_of::<usize>();
    unsafe {
        let _ = jemalloc_sys::mallctl(
            name.as_ptr().cast(),
            (&mut value as *mut usize).cast(),
            &mut size,
            null_mut(),
            0,
        );
    }
    value
}

#[inline]
fn read_u32(name: &[u8]) -> u32 {
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>();
    unsafe {
        let _ = jemalloc_sys::mallctl(
            name.as_ptr().cast(),
            (&mut value as *mut u32).cast(),
            &mut size,
            null_mut(),
            0,
        );
    }
    value
}

/// Raw allocation helper. The caller must free with `dealloc_raw` using the
/// same layout after all EBR readers have left the object.
pub unsafe fn alloc_raw(layout: Layout) -> *mut u8 {
    unsafe { Zmalloc.alloc(layout) }
}

pub unsafe fn dealloc_raw(ptr: *mut u8, layout: Layout) {
    if !ptr.is_null() {
        unsafe {
            Zmalloc.dealloc(ptr, layout);
        }
    }
}

/// Allocate a long-lived object without jemalloc's thread cache. This mirrors
/// Redis's `zmalloc_no_tcache` path and makes freed persistent objects visible
/// to decay/purge promptly. Alignment is preserved for layouts requiring more
/// than the platform default.
pub unsafe fn alloc_raw_no_tcache(layout: Layout) -> *mut u8 {
    let mut flags = jemalloc_sys::MALLOCX_TCACHE_NONE;
    if layout.align() > std::mem::align_of::<usize>() {
        flags |= jemalloc_sys::MALLOCX_ALIGN(layout.align());
    }
    unsafe { jemalloc_sys::mallocx(layout.size().max(1), flags).cast() }
}

pub unsafe fn dealloc_raw_no_tcache(ptr: *mut u8, layout: Layout) {
    if !ptr.is_null() {
        let mut flags = jemalloc_sys::MALLOCX_TCACHE_NONE;
        if layout.align() > std::mem::align_of::<usize>() {
            flags |= jemalloc_sys::MALLOCX_ALIGN(layout.align());
        }
        // `sdallocx` requires the allocator's usable size, which is not the
        // same as every Rust `Layout` request. `dallocx` avoids size-class
        // mismatches while retaining the no-tcache behavior.
        let _ = layout;
        unsafe {
            jemalloc_sys::dallocx(ptr.cast(), flags);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_no_tcache_preserves_alignment_and_contents() {
        for &(size, align) in &[(1, 1), (17, 8), (129, 64), (4096, 4096)] {
            let layout = Layout::from_size_align(size, align).unwrap();
            let ptr = unsafe { alloc_raw_no_tcache(layout) };
            assert!(!ptr.is_null());
            assert_eq!((ptr as usize) % align, 0);
            unsafe {
                std::ptr::write_bytes(ptr, 0xA5, size);
                assert_eq!(*ptr, 0xA5);
                assert_eq!(*ptr.add(size - 1), 0xA5);
                dealloc_raw_no_tcache(ptr, layout);
            }
        }
    }

    #[test]
    fn allocator_stats_are_coherent() {
        let stats = stats();
        assert!(stats.active >= stats.allocated);
        assert!(stats.resident >= stats.active);
    }
}
