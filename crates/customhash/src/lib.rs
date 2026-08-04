//! CustomMap — fully lock-free sharded concurrent hash map.
//!
//! Generic over value type V. The lock-free invariants:
//!   1. Entry slots are write-once (null → entry), never freed while map lives.
//!   2. Values are swapped atomically; old values retired via epoch-based reclamation.
//!   3. Slot array never reallocates — fixed capacity, sized at construction.

use std::marker::PhantomData;
use std::ops::Deref;
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

use crossbeam_utils::CachePadded;
use foldhash::fast::RandomState;
use std::hash::BuildHasher;

// ─────────────────────────────────────────────────────────────────────────────
// Epoch-based reclamation
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) mod ebr;

// ─────────────────────────────────────────────────────────────────────────────
// ValueBox — the only mutable, reclaimed allocation (generic over V)
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) struct ValueBox<V>(pub V);

#[inline]
pub(crate) fn new_value<V>(v: V) -> *mut ValueBox<V> {
    Box::into_raw(Box::new(ValueBox(v)))
}

/// # Safety
/// `p` must come from [`new_value`] and must not have been freed.
pub(crate) unsafe fn free_value<V>(p: *mut ValueBox<V>) {
    drop(unsafe { Box::from_raw(p) });
}

// ─────────────────────────────────────────────────────────────────────────────
// Entry — immutable except for `value`; never freed while the map lives
// ─────────────────────────────────────────────────────────────────────────────

struct Entry<V> {
    hash: u64,
    value: AtomicPtr<ValueBox<V>>,
    key: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Shard — fixed-capacity open-addressed table, linear probing
// ─────────────────────────────────────────────────────────────────────────────

const LOAD_NUM: usize = 7;
const LOAD_DEN: usize = 10;
const DEFAULT_SHARD_CAPACITY: usize = 32_768;

#[repr(align(128))]
struct Shard<V> {
    slots: Box<[AtomicPtr<Entry<V>>]>,
    mask: usize,
    len: CachePadded<AtomicUsize>,
    threshold: usize,
}

impl<V> Shard<V> {
    fn with_capacity(cap: usize) -> Self {
        let cap = cap.next_power_of_two().max(8);
        let slots = (0..cap)
            .map(|_| AtomicPtr::new(ptr::null_mut()))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Shard {
            slots,
            mask: cap - 1,
            len: CachePadded::new(AtomicUsize::new(0)),
            threshold: cap * LOAD_NUM / LOAD_DEN,
        }
    }

    #[inline(always)]
    fn find(&self, key: &str, hash: u64) -> Option<&Entry<V>> {
        let mut i = (hash as usize) & self.mask;
        loop {
            let p = unsafe { self.slots.get_unchecked(i) }.load(Ordering::Acquire);
            if p.is_null() {
                return None;
            }
            let e = unsafe { &*p };
            if e.hash == hash && e.key == key {
                return Some(e);
            }
            i = (i + 1) & self.mask;
        }
    }

    /// Insert or update. Returns Ok(true) if new, Ok(false) if updated, Err(Full) if saturated.
    #[inline(always)]
    fn insert_hashed(&self, key: String, value: V, hash: u64) -> Result<bool, Full> {
        // Fast path: key already exists, just swap the value (single TLS access)
        if let Some(existing) = self.find(&key, hash) {
            let is_new = ebr::replace_value(&existing.value, value);
            return Ok(is_new);
        }

        self.insert_new_hashed(key, value, hash)
    }

    #[cold]
    #[inline(never)]
    fn insert_new_hashed(&self, key: String, value: V, hash: u64) -> Result<bool, Full> {
        if self.len.load(Ordering::Relaxed) >= self.threshold {
            return Err(Full);
        }

        let vb = new_value(value);
        let entry: *mut Entry<V> = Box::into_raw(Box::new(Entry {
            hash,
            value: AtomicPtr::new(vb),
            key,
        }));
        let key_ref: &str = unsafe { &(*entry).key };
        let mut reserved = false;

        let mut i = (hash as usize) & self.mask;
        loop {
            let slot = unsafe { self.slots.get_unchecked(i) };
            let p = slot.load(Ordering::Acquire);

            if p.is_null() {
                if !reserved {
                    let mut cur = self.len.load(Ordering::Relaxed);
                    loop {
                        if cur >= self.threshold {
                            unsafe {
                                drop(Box::from_raw(entry));
                                free_value(vb);
                            }
                            return Err(Full);
                        }
                        match self.len.compare_exchange_weak(
                            cur,
                            cur + 1,
                            Ordering::Relaxed,
                            Ordering::Relaxed,
                        ) {
                            Ok(_) => break,
                            Err(observed) => cur = observed,
                        }
                    }
                    reserved = true;
                }

                if slot
                    .compare_exchange(ptr::null_mut(), entry, Ordering::Release, Ordering::Acquire)
                    .is_ok()
                {
                    return Ok(true);
                }
                continue;
            }

            let e = unsafe { &*p };
            if e.hash == hash && e.key == key_ref {
                // Another thread inserted this key — update instead
                let old = e.value.swap(vb, Ordering::AcqRel);
                if reserved {
                    self.len.fetch_sub(1, Ordering::Relaxed);
                }
                unsafe { drop(Box::from_raw(entry)) };
                if !old.is_null() {
                    unsafe { ebr::retire_value(old) };
                }
                return Ok(old.is_null());
            }

            i = (i + 1) & self.mask;
        }
    }

    /// Iterate all live entries
    fn for_each<F>(&self, mut f: F)
    where
        F: FnMut(&str, &Entry<V>),
    {
        for slot in self.slots.iter() {
            let p = slot.load(Ordering::Acquire);
            if p.is_null() {
                continue;
            }
            let e = unsafe { &*p };
            if !e.value.load(Ordering::Acquire).is_null() {
                f(&e.key, e);
            }
        }
    }
}

impl<V> Drop for Shard<V> {
    fn drop(&mut self) {
        for slot in self.slots.iter() {
            let p = slot.load(Ordering::Relaxed);
            if p.is_null() {
                continue;
            }
            let entry = unsafe { Box::from_raw(p) };
            let v = entry.value.load(Ordering::Relaxed);
            if !v.is_null() {
                unsafe { free_value(v) };
            }
        }
    }
}

unsafe impl<V: Send> Sync for Shard<V> {}
unsafe impl<V: Send> Send for Shard<V> {}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Full;

impl std::fmt::Display for Full {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CustomMap shard is full (fixed capacity, no resize)")
    }
}
impl std::error::Error for Full {}

/// A zero-copy value reference protected by an epoch guard.
pub struct ValueRef<'a, V> {
    ptr: *const ValueBox<V>,
    _guard: ebr::Guard,
    _map: PhantomData<&'a CustomMap<V>>,
}

impl<V> Deref for ValueRef<'_, V> {
    type Target = V;

    #[inline(always)]
    fn deref(&self) -> &V {
        unsafe { &(*self.ptr).0 }
    }
}

impl<V: std::fmt::Debug> std::fmt::Debug for ValueRef<'_, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, f)
    }
}

pub struct CustomMap<V> {
    shards: Box<[Shard<V>]>,
    shift: u32,
    shard_mask: usize,
    hasher: RandomState,
    key_count: CachePadded<AtomicUsize>,
}

impl<V: Clone + Send + Sync> CustomMap<V> {
    pub fn with_shards(shard_count: usize) -> Self {
        Self::build(shard_count.next_power_of_two().max(1), DEFAULT_SHARD_CAPACITY)
    }

    pub fn with_capacity(shard_count: usize, expected_keys: usize) -> Self {
        let n = shard_count.next_power_of_two().max(1);
        let keys_per_shard = expected_keys.div_ceil(n);
        let at_load_limit = keys_per_shard.saturating_mul(LOAD_DEN).div_ceil(LOAD_NUM);
        let per_shard = at_load_limit.saturating_mul(5).div_ceil(4);
        Self::build(n, per_shard.max(8))
    }

    fn build(shards: usize, per_shard: usize) -> Self {
        let shift = (u64::BITS - shards.trailing_zeros()).min(63);
        let shard_mask = shards - 1;
        let shards = (0..shards)
            .map(|_| Shard::with_capacity(per_shard))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        CustomMap {
            shards,
            shift,
            shard_mask,
            hasher: RandomState::default(),
            key_count: CachePadded::new(AtomicUsize::new(0)),
        }
    }

    #[inline(always)]
    fn locate(&self, key: &str) -> (u64, usize) {
        let h = self.hasher.hash_one(key);
        (h, ((h >> self.shift) as usize) & self.shard_mask)
    }

    /// Expose hash + shard index for callers that want to avoid re-hashing.
    #[inline(always)]
    pub fn locate_key(&self, key: &str) -> (u64, usize) {
        self.locate(key)
    }

    /// Apply a closure to an entry's value with a single pin/unpin (one TLS access).
    /// Returns None if key not found or value is tombstoned.
    #[inline]
    pub fn with_entry<R>(&self, key: &str, hash: u64, shard_idx: usize, f: impl FnOnce(&V) -> R) -> Option<R> {
        let shard = unsafe { self.shards.get_unchecked(shard_idx) };
        let entry = shard.find(key, hash)?;
        ebr::with_pin(|_| {
            let ptr = entry.value.load(Ordering::Acquire);
            if ptr.is_null() {
                return None;
            }
            Some(f(unsafe { &(*ptr).0 }))
        })
    }

    /// Wait-free contains check — no TLS, no pin needed.
    #[inline]
    pub fn contains_key(&self, key: &str) -> bool {
        let (h, idx) = self.locate(key);
        unsafe { self.shards.get_unchecked(idx) }
            .find(key, h)
            .is_some_and(|entry| !entry.value.load(Ordering::Acquire).is_null())
    }

    /// Lock-free get — clones the value. Single TLS access (pin+clone+unpin).
    #[inline]
    pub fn get(&self, key: &str) -> Option<V> {
        let (h, idx) = self.locate(key);
        let e = unsafe { self.shards.get_unchecked(idx) }.find(key, h)?;
        ebr::read_clone(&e.value)
    }

    /// Zero-copy read — returns a reference protected by epoch guard.
    #[inline]
    pub fn get_ref(&self, key: &str) -> Option<ValueRef<'_, V>> {
        let (h, idx) = self.locate(key);
        let guard = ebr::pin::<V>();
        let e = unsafe { self.shards.get_unchecked(idx) }.find(key, h)?;
        let ptr = e.value.load(Ordering::Acquire);
        if ptr.is_null() {
            return None;
        }
        Some(ValueRef {
            ptr,
            _guard: guard,
            _map: PhantomData,
        })
    }

    /// Lock-free — apply a function to the value in-place (zero-copy read).
    #[inline]
    pub fn with_value<R>(&self, key: &str, f: impl FnOnce(&V) -> R) -> Option<R> {
        self.get_ref(key).map(|v| f(&v))
    }

    /// Lock-free insert. Returns true if new key, false if updated existing.
    #[inline]
    pub fn insert(&self, key: String, value: V) -> bool {
        match self.try_insert(key, value) {
            Ok(is_new) => is_new,
            Err(Full) => panic!("CustomMap shard full"),
        }
    }

    #[inline]
    pub fn try_insert(&self, key: String, value: V) -> Result<bool, Full> {
        let (h, idx) = self.locate(&key);
        let is_new = unsafe { self.shards.get_unchecked(idx) }.insert_hashed(key, value, h)?;
        if is_new {
            self.key_count.fetch_add(1, Ordering::Relaxed);
        }
        Ok(is_new)
    }

    /// Optimized SET: avoids key allocation when key already exists.
    /// `key_owned` is only called if a new entry must be created.
    #[inline]
    pub fn set(&self, key: &str, value: V, key_owned: impl FnOnce() -> String) -> bool {
        let (h, idx) = self.locate(key);
        let shard = unsafe { self.shards.get_unchecked(idx) };
        // Hot path: key exists, just swap value (no key allocation, single TLS)
        if let Some(existing) = shard.find(key, h) {
            let is_new = ebr::replace_value(&existing.value, value);
            if is_new {
                self.key_count.fetch_add(1, Ordering::Relaxed);
            }
            return is_new;
        }
        // Cold path: new key
        let key_string = key_owned();
        match shard.insert_new_hashed(key_string, value, h) {
            Ok(true) => {
                self.key_count.fetch_add(1, Ordering::Relaxed);
                true
            }
            Ok(false) => false,
            Err(Full) => panic!("CustomMap shard full"),
        }
    }

    /// Lock-free remove via atomic null swap.
    #[inline]
    pub fn remove(&self, key: &str) -> Option<V> {
        let (h, idx) = self.locate(key);
        let shard = unsafe { self.shards.get_unchecked(idx) };
        let entry = shard.find(key, h)?;
        let _guard = ebr::pin::<V>();
        let old = entry.value.swap(ptr::null_mut(), Ordering::AcqRel);
        if old.is_null() {
            return None;
        }
        let value = unsafe { (*old).0.clone() };
        self.key_count.fetch_sub(1, Ordering::Relaxed);
        unsafe { ebr::retire_value(old) };
        Some(value)
    }

    /// Atomically read-modify-write a value.
    #[inline]
    pub fn update<R>(&self, key: &str, f: impl FnOnce(&V) -> (V, R)) -> Option<R> {
        let (h, idx) = self.locate(key);
        let _guard = ebr::pin::<V>();
        let shard = unsafe { self.shards.get_unchecked(idx) };
        let entry = shard.find(key, h)?;
        let old_ptr = entry.value.load(Ordering::Acquire);
        if old_ptr.is_null() {
            return None;
        }
        let (new_val, result) = f(unsafe { &(*old_ptr).0 });
        let new_ptr = new_value(new_val);
        let swapped = entry.value.swap(new_ptr, Ordering::AcqRel);
        if !swapped.is_null() {
            unsafe { ebr::retire_value(swapped) };
        }
        Some(result)
    }

    /// Like update but the closure can fail.
    #[inline]
    pub fn try_update<R>(&self, key: &str, f: impl FnOnce(&V) -> Option<(V, R)>) -> Option<R> {
        let (h, idx) = self.locate(key);
        let _guard = ebr::pin::<V>();
        let shard = unsafe { self.shards.get_unchecked(idx) };
        let entry = shard.find(key, h)?;
        let old_ptr = entry.value.load(Ordering::Acquire);
        if old_ptr.is_null() {
            return None;
        }
        let (new_val, result) = f(unsafe { &(*old_ptr).0 })?;
        let new_ptr = new_value(new_val);
        let swapped = entry.value.swap(new_ptr, Ordering::AcqRel);
        if !swapped.is_null() {
            unsafe { ebr::retire_value(swapped) };
        }
        Some(result)
    }

    /// Insert only if key does not exist. Returns true if inserted.
    #[inline]
    pub fn insert_if_absent(&self, key: String, value: V) -> bool {
        let (h, idx) = self.locate(&key);
        let shard = unsafe { self.shards.get_unchecked(idx) };

        if let Some(entry) = shard.find(&key, h) {
            if !entry.value.load(Ordering::Acquire).is_null() {
                return false;
            }
            let new_ptr = new_value(value);
            match entry.value.compare_exchange(
                ptr::null_mut(),
                new_ptr,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.key_count.fetch_add(1, Ordering::Relaxed);
                    true
                }
                Err(_) => {
                    unsafe { free_value(new_ptr) };
                    false
                }
            }
        } else {
            match shard.insert_new_hashed(key, value, h) {
                Ok(true) => {
                    self.key_count.fetch_add(1, Ordering::Relaxed);
                    true
                }
                _ => false,
            }
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.key_count.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterate all live key-value pairs under epoch protection.
    pub fn for_each<F>(&self, mut f: F)
    where
        F: FnMut(&str, &V),
    {
        let _guard = ebr::pin::<V>();
        for shard in self.shards.iter() {
            shard.for_each(|key, entry| {
                let ptr = entry.value.load(Ordering::Acquire);
                if !ptr.is_null() {
                    f(key, unsafe { &(*ptr).0 });
                }
            });
        }
    }

    /// Collect all keys.
    pub fn keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        let _guard = ebr::pin::<V>();
        for shard in self.shards.iter() {
            shard.for_each(|key, _| {
                keys.push(key.to_string());
            });
        }
        keys
    }

    /// Retain only entries where `f` returns true.
    pub fn retain<F>(&self, mut f: F)
    where
        F: FnMut(&str, &V) -> bool,
    {
        let _guard = ebr::pin::<V>();
        for shard in self.shards.iter() {
            for slot in shard.slots.iter() {
                let p = slot.load(Ordering::Acquire);
                if p.is_null() {
                    continue;
                }
                let entry = unsafe { &*p };
                let vptr = entry.value.load(Ordering::Acquire);
                if vptr.is_null() {
                    continue;
                }
                let val = unsafe { &(*vptr).0 };
                if !f(&entry.key, val) {
                    let old = entry.value.swap(ptr::null_mut(), Ordering::AcqRel);
                    if !old.is_null() {
                        self.key_count.fetch_sub(1, Ordering::Relaxed);
                        unsafe { ebr::retire_value(old) };
                    }
                }
            }
        }
    }

    /// Clear all entries.
    pub fn clear(&self) {
        let _guard = ebr::pin::<V>();
        for shard in self.shards.iter() {
            for slot in shard.slots.iter() {
                let p = slot.load(Ordering::Acquire);
                if p.is_null() {
                    continue;
                }
                let entry = unsafe { &*p };
                let old = entry.value.swap(ptr::null_mut(), Ordering::AcqRel);
                if !old.is_null() {
                    self.key_count.fetch_sub(1, Ordering::Relaxed);
                    unsafe { ebr::retire_value(old) };
                }
            }
        }
    }
}

unsafe impl<V: Send + Sync> Sync for CustomMap<V> {}
unsafe impl<V: Send + Sync> Send for CustomMap<V> {}
