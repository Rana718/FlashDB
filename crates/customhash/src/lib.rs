mod ebr;

use std::marker::PhantomData;
use std::ops::Deref;
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

use crossbeam_utils::CachePadded;
use foldhash::fast::RandomState;
use std::hash::BuildHasher;

pub(crate) struct ValueBox<V>(pub V);

#[inline]
pub(crate) fn new_value<V>(v: V) -> *mut ValueBox<V> {
    Box::into_raw(Box::new(ValueBox(v)))
}

#[inline]
pub(crate) unsafe fn free_value<V>(p: *mut ValueBox<V>) {
    unsafe { drop(Box::from_raw(p)) };
}

struct Entry<V> {
    hash: u64,
    value: AtomicPtr<ValueBox<V>>,
    key: String,
}

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

impl<V: 'static> Shard<V> {
    fn new(cap: usize) -> Self {
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

    #[inline(always)]
    fn insert_hashed(&self, key: String, value: V, hash: u64) -> Result<bool, Full> {
        if let Some(existing) = self.find(&key, hash) {
            let is_new = ebr::replace_value(&existing.value, value);
            return Ok(is_new);
        }
        self.insert_new(key, value, hash)
    }

    #[cold]
    #[inline(never)]
    fn insert_new(&self, key: String, value: V, hash: u64) -> Result<bool, Full> {
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
                            Err(observed) => {
                                cur = observed;
                                std::hint::spin_loop();
                            }
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
}

impl<V> Drop for Shard<V> {
    fn drop(&mut self) {
        for slot in self.slots.iter() {
            let p = slot.load(Ordering::Relaxed);
            if !p.is_null() {
                let entry = unsafe { Box::from_raw(p) };
                let v = entry.value.load(Ordering::Relaxed);
                if !v.is_null() {
                    unsafe { free_value(v) };
                }
            }
        }
    }
}

unsafe impl<V: Send> Sync for Shard<V> {}
unsafe impl<V: Send> Send for Shard<V> {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Full;

impl std::fmt::Display for Full {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("shard full")
    }
}
impl std::error::Error for Full {}

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

pub struct CustomMap<V> {
    shards: Box<[Shard<V>]>,
    shift: u32,
    shard_mask: usize,
    hasher: RandomState,
    key_count: CachePadded<AtomicUsize>,
}

impl<V: Clone + Send + Sync + 'static> CustomMap<V> {
    pub fn with_shards(n: usize) -> Self {
        Self::build(n.next_power_of_two().max(1), DEFAULT_SHARD_CAPACITY)
    }

    pub fn with_capacity(shard_count: usize, expected_keys: usize) -> Self {
        let n = shard_count.next_power_of_two().max(1);
        let per = expected_keys.div_ceil(n);
        let at_limit = per.saturating_mul(LOAD_DEN).div_ceil(LOAD_NUM);
        let per_shard = at_limit.saturating_mul(5).div_ceil(4).max(8);
        Self::build(n, per_shard)
    }

    fn build(n: usize, per_shard: usize) -> Self {
        CustomMap {
            shift: (u64::BITS - n.trailing_zeros()).min(63),
            shard_mask: n - 1,
            shards: (0..n)
                .map(|_| Shard::new(per_shard))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            hasher: RandomState::default(),
            key_count: CachePadded::new(AtomicUsize::new(0)),
        }
    }

    #[inline(always)]
    fn locate(&self, key: &str) -> (u64, usize) {
        let h = self.hasher.hash_one(key);
        (h, ((h >> self.shift) as usize) & self.shard_mask)
    }

    #[inline(always)]
    pub fn locate_key(&self, key: &str) -> (u64, usize) {
        self.locate(key)
    }

    #[inline]
    pub fn with_entry<R>(
        &self,
        key: &str,
        hash: u64,
        idx: usize,
        f: impl FnOnce(&V) -> R,
    ) -> Option<R> {
        let entry = unsafe { self.shards.get_unchecked(idx) }.find(key, hash)?;
        ebr::with_pin(|_| {
            let ptr = entry.value.load(Ordering::Acquire);
            if ptr.is_null() {
                return None;
            }
            Some(f(unsafe { &(*ptr).0 }))
        })
    }

    #[inline]
    pub fn contains_key(&self, key: &str) -> bool {
        let (h, idx) = self.locate(key);
        unsafe { self.shards.get_unchecked(idx) }
            .find(key, h)
            .is_some_and(|e| !e.value.load(Ordering::Acquire).is_null())
    }

    #[inline]
    pub fn get(&self, key: &str) -> Option<V> {
        let (h, idx) = self.locate(key);
        let e = unsafe { self.shards.get_unchecked(idx) }.find(key, h)?;
        ebr::read_clone(&e.value)
    }

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

    #[inline]
    pub fn insert(&self, key: String, value: V) -> bool {
        self.try_insert(key, value)
            .unwrap_or_else(|_| panic!("shard full"))
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

    #[inline]
    pub fn set(&self, key: &str, value: V, key_owned: impl FnOnce() -> String) -> bool {
        let (h, idx) = self.locate(key);
        let shard = unsafe { self.shards.get_unchecked(idx) };
        if let Some(existing) = shard.find(key, h) {
            let is_new = ebr::replace_value(&existing.value, value);
            if is_new {
                self.key_count.fetch_add(1, Ordering::Relaxed);
            }
            return is_new;
        }
        match shard.insert_new(key_owned(), value, h) {
            Ok(true) => {
                self.key_count.fetch_add(1, Ordering::Relaxed);
                true
            }
            Ok(false) => false,
            Err(_) => panic!("shard full"),
        }
    }

    #[inline]
    pub fn remove(&self, key: &str) -> Option<V> {
        let (h, idx) = self.locate(key);
        let entry = unsafe { self.shards.get_unchecked(idx) }.find(key, h)?;
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

    #[inline]
    pub fn update<R>(&self, key: &str, mut f: impl FnMut(&V) -> (V, R)) -> Option<R> {
        let (h, idx) = self.locate(key);
        let _guard = ebr::pin::<V>();
        let entry = unsafe { self.shards.get_unchecked(idx) }.find(key, h)?;
        loop {
            let old_ptr = entry.value.load(Ordering::Acquire);
            if old_ptr.is_null() {
                return None;
            }
            let (new_val, result) = f(unsafe { &(*old_ptr).0 });
            let new_ptr = new_value(new_val);
            match entry.value.compare_exchange(
                old_ptr,
                new_ptr,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    unsafe { ebr::retire_value(old_ptr) };
                    return Some(result);
                }
                Err(_) => unsafe { free_value(new_ptr) },
            }
        }
    }

    #[inline]
    pub fn try_update<R>(&self, key: &str, mut f: impl FnMut(&V) -> Option<(V, R)>) -> Option<R> {
        let (h, idx) = self.locate(key);
        let _guard = ebr::pin::<V>();
        let entry = unsafe { self.shards.get_unchecked(idx) }.find(key, h)?;
        loop {
            let old_ptr = entry.value.load(Ordering::Acquire);
            if old_ptr.is_null() {
                return None;
            }
            let (new_val, result) = f(unsafe { &(*old_ptr).0 })?;
            let new_ptr = new_value(new_val);
            match entry.value.compare_exchange(
                old_ptr,
                new_ptr,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    unsafe { ebr::retire_value(old_ptr) };
                    return Some(result);
                }
                Err(_) => unsafe { free_value(new_ptr) },
            }
        }
    }

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
            match shard.insert_new(key, value, h) {
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

    pub fn for_each(&self, mut f: impl FnMut(&str, &V)) {
        let _guard = ebr::pin::<V>();
        for shard in self.shards.iter() {
            for slot in shard.slots.iter() {
                let p = slot.load(Ordering::Acquire);
                if p.is_null() {
                    continue;
                }
                let e = unsafe { &*p };
                let vptr = e.value.load(Ordering::Acquire);
                if !vptr.is_null() {
                    f(&e.key, unsafe { &(*vptr).0 });
                }
            }
        }
    }

    pub fn keys(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.for_each(|k, _| out.push(k.to_owned()));
        out
    }

    pub fn retain(&self, mut f: impl FnMut(&str, &V) -> bool) {
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
                if !f(&entry.key, unsafe { &(*vptr).0 }) {
                    let old = entry.value.swap(ptr::null_mut(), Ordering::AcqRel);
                    if !old.is_null() {
                        self.key_count.fetch_sub(1, Ordering::Relaxed);
                        unsafe { ebr::retire_value(old) };
                    }
                }
            }
        }
    }

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
