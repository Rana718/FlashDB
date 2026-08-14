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
const INITIAL_SHARD_CAPACITY: usize = 1024;
const GROWING: usize = 1usize << (usize::BITS - 1);

struct SlotTable<V> {
    slots: Box<[AtomicPtr<Entry<V>>]>,
    mask: usize,
    threshold: usize,
}

impl<V> SlotTable<V> {
    fn new(cap: usize) -> Self {
        let cap = cap.next_power_of_two().max(8);
        let slots = (0..cap)
            .map(|_| AtomicPtr::new(ptr::null_mut()))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        SlotTable {
            slots,
            mask: cap - 1,
            threshold: cap * LOAD_NUM / LOAD_DEN,
        }
    }

    fn capacity(&self) -> usize {
        self.slots.len()
    }
}

#[repr(align(128))]
struct Shard<V> {
    table: AtomicPtr<SlotTable<V>>,
    len: CachePadded<AtomicUsize>,
    insert_gate: CachePadded<AtomicUsize>,
    grow_lock: std::sync::Mutex<()>,
}

struct InsertGuard<'a> {
    gate: &'a AtomicUsize,
}

impl Drop for InsertGuard<'_> {
    #[inline(always)]
    fn drop(&mut self) {
        self.gate.fetch_sub(1, Ordering::Release);
    }
}

impl<V: Clone + Send + Sync + 'static> Shard<V> {
    fn new(cap: usize) -> Self {
        let table = Box::into_raw(Box::new(SlotTable::new(cap)));
        Shard {
            table: AtomicPtr::new(table),
            len: CachePadded::new(AtomicUsize::new(0)),
            insert_gate: CachePadded::new(AtomicUsize::new(0)),
            grow_lock: std::sync::Mutex::new(()),
        }
    }

    #[inline(always)]
    fn table(&self) -> &SlotTable<V> {
        unsafe { &*self.table.load(Ordering::Acquire) }
    }

    #[inline(always)]
    fn enter_insert(&self) -> InsertGuard<'_> {
        loop {
            let state = self.insert_gate.load(Ordering::Relaxed);
            if state & GROWING != 0 {
                std::hint::spin_loop();
                continue;
            }
            if self
                .insert_gate
                .compare_exchange_weak(state, state + 1, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return InsertGuard {
                    gate: &self.insert_gate,
                };
            }
        }
    }

    #[inline(always)]
    fn find(&self, key: &str, hash: u64) -> Option<&Entry<V>> {
        let t = self.table();
        let mut i = (hash as usize) & t.mask;
        loop {
            let p = unsafe { t.slots.get_unchecked(i) }.load(Ordering::Acquire);
            if p.is_null() {
                return None;
            }
            let e = unsafe { &*p };
            if e.hash == hash && e.key == key {
                return Some(e);
            }
            i = (i + 1) & t.mask;
        }
    }

    #[inline(always)]
    fn insert_hashed(&self, key: String, value: V, hash: u64) -> bool {
        let _guard = ebr::pin::<V>();
        if let Some(existing) = self.find(&key, hash) {
            let is_new = ebr::replace_value(&existing.value, value);
            return is_new;
        }
        self.insert_new(key, value, hash)
    }

    fn insert_new(&self, key: String, value: V, hash: u64) -> bool {
        let vb = new_value(value);
        let entry: *mut Entry<V> = Box::into_raw(Box::new(Entry {
            hash,
            value: AtomicPtr::new(vb),
            key,
        }));
        let key_ref: &str = unsafe { &(*entry).key };

        loop {
            let insert_guard = self.enter_insert();
            let t = self.table();
            if self.len.load(Ordering::Relaxed) >= t.threshold {
                drop(insert_guard);
                self.grow();
                continue;
            }
            let mut reserved = false;

            let t = self.table();
            let mut i = (hash as usize) & t.mask;
            loop {
                let slot = unsafe { t.slots.get_unchecked(i) };
                let p = slot.load(Ordering::Acquire);

                if p.is_null() {
                    if !reserved {
                        let cur = self.len.load(Ordering::Relaxed);
                        if cur >= t.threshold {
                            drop(insert_guard);
                            self.grow();
                            break;
                        }
                        match self.len.compare_exchange_weak(
                            cur,
                            cur + 1,
                            Ordering::Relaxed,
                            Ordering::Relaxed,
                        ) {
                            Ok(_) => {
                                reserved = true;
                            }
                            Err(_) => {
                                std::hint::spin_loop();
                                continue;
                            }
                        }
                    }

                    if slot
                        .compare_exchange(
                            ptr::null_mut(),
                            entry,
                            Ordering::Release,
                            Ordering::Acquire,
                        )
                        .is_ok()
                    {
                        return true;
                    }
                    continue;
                }

                let e = unsafe { &*p };
                if e.hash == hash && e.key == key_ref {
                    let old = e.value.swap(vb, Ordering::AcqRel);
                    if reserved {
                        self.len.fetch_sub(1, Ordering::Relaxed);
                    }
                    unsafe {
                        free_value(vb);
                        let _ = Box::from_raw(entry);
                    }
                    if !old.is_null() {
                        unsafe { ebr::retire_value(old) };
                    }
                    return old.is_null();
                }

                i = (i + 1) & t.mask;
            }
        }
    }

    fn grow(&self) {
        let _lock = self.grow_lock.lock().unwrap_or_else(|e| e.into_inner());

        while self
            .insert_gate
            .compare_exchange_weak(0, GROWING, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            std::hint::spin_loop();
        }
        self.grow_locked();
        self.insert_gate.store(0, Ordering::Release);
    }

    fn grow_locked(&self) {
        let old_ptr = self.table.load(Ordering::Acquire);
        let old_table = unsafe { &*old_ptr };
        let cur_len = self.len.load(Ordering::Relaxed);

        if cur_len < old_table.threshold {
            return;
        }

        let new_cap = (old_table.capacity() * 2).next_power_of_two();
        let new_table = Box::new(SlotTable::<V>::new(new_cap));

        for slot in old_table.slots.iter() {
            let p = slot.load(Ordering::Acquire);
            if p.is_null() {
                continue;
            }
            let e = unsafe { &*p };
            if e.value.load(Ordering::Acquire).is_null() {
                continue;
            }

            let mut i = (e.hash as usize) & new_table.mask;
            loop {
                let new_slot = unsafe { new_table.slots.get_unchecked(i) };
                if new_slot.load(Ordering::Relaxed).is_null() {
                    new_slot.store(p, Ordering::Relaxed);
                    break;
                }
                i = (i + 1) & new_table.mask;
            }
        }

        let new_ptr = Box::into_raw(new_table);
        self.table.store(new_ptr, Ordering::Release);
        unsafe { ebr::retire_box(old_ptr) };
    }
}

impl<V> Drop for Shard<V> {
    fn drop(&mut self) {
        let t_ptr = self.table.load(Ordering::Relaxed);
        if !t_ptr.is_null() {
            let t = unsafe { &*t_ptr };
            for slot in t.slots.iter() {
                let p = slot.load(Ordering::Relaxed);
                if !p.is_null() {
                    let entry = unsafe { Box::from_raw(p) };
                    let v = entry.value.load(Ordering::Relaxed);
                    if !v.is_null() {
                        unsafe { free_value(v) };
                    }
                }
            }
            unsafe {
                drop(Box::from_raw(t_ptr));
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
        f.write_str("OOM command not allowed: store is at capacity")
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
        let per_shard = at_limit.max(INITIAL_SHARD_CAPACITY);
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
        ebr::with_pin(|_| {
            let entry = unsafe { self.shards.get_unchecked(idx) }.find(key, hash)?;
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
        ebr::with_pin(|_| {
            unsafe { self.shards.get_unchecked(idx) }
                .find(key, h)
                .is_some_and(|e| !e.value.load(Ordering::Acquire).is_null())
        })
    }

    #[inline]
    pub fn get(&self, key: &str) -> Option<V> {
        let (h, idx) = self.locate(key);
        ebr::with_pin(|_| {
            let e = unsafe { self.shards.get_unchecked(idx) }.find(key, h)?;
            let ptr = e.value.load(Ordering::Acquire);
            (!ptr.is_null()).then(|| unsafe { (*ptr).0.clone() })
        })
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
        let (h, idx) = self.locate(&key);
        let is_new = unsafe { self.shards.get_unchecked(idx) }.insert_hashed(key, value, h);
        if is_new {
            self.key_count.fetch_add(1, Ordering::Relaxed);
        }
        is_new
    }

    #[inline]
    pub fn try_insert(&self, key: String, value: V) -> Result<bool, Full> {
        Ok(self.insert(key, value))
    }

    #[inline]
    pub fn set(&self, key: &str, value: V, key_owned: impl FnOnce() -> String) -> bool {
        let (h, idx) = self.locate(key);
        let _guard = ebr::pin::<V>();
        let shard = unsafe { self.shards.get_unchecked(idx) };
        if let Some(existing) = shard.find(key, h) {
            let is_new = ebr::replace_value(&existing.value, value);
            if is_new {
                self.key_count.fetch_add(1, Ordering::Relaxed);
            }
            return is_new;
        }
        let is_new = shard.insert_new(key_owned(), value, h);
        if is_new {
            self.key_count.fetch_add(1, Ordering::Relaxed);
        }
        is_new
    }

    #[inline]
    pub fn try_set(
        &self,
        key: &str,
        value: V,
        key_owned: impl FnOnce() -> String,
    ) -> Result<bool, Full> {
        Ok(self.set(key, value, key_owned))
    }

    #[inline]
    pub fn remove(&self, key: &str) -> Option<V> {
        let (h, idx) = self.locate(key);
        let _guard = ebr::pin::<V>();
        let entry = unsafe { self.shards.get_unchecked(idx) }.find(key, h)?;
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
        let _guard = ebr::pin::<V>();
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
            let is_new = shard.insert_new(key, value, h);
            if is_new {
                self.key_count.fetch_add(1, Ordering::Relaxed);
            }
            is_new
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
            let t = shard.table();
            for slot in t.slots.iter() {
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
            let t = shard.table();
            for slot in t.slots.iter() {
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

    /// Retain entries in one shard.
    pub fn retain_shard(&self, shard_idx: usize, mut f: impl FnMut(&str, &V) -> bool) {
        let _guard = ebr::pin::<V>();
        let Some(shard) = self.shards.get(shard_idx) else {
            return;
        };
        let t = shard.table();
        for slot in t.slots.iter() {
            let p = slot.load(Ordering::Acquire);
            if p.is_null() {
                continue;
            }
            let entry = unsafe { &*p };
            let vptr = entry.value.load(Ordering::Acquire);
            if vptr.is_null() || f(&entry.key, unsafe { &(*vptr).0 }) {
                continue;
            }
            let old = entry.value.swap(ptr::null_mut(), Ordering::AcqRel);
            if !old.is_null() {
                self.key_count.fetch_sub(1, Ordering::Relaxed);
                unsafe { ebr::retire_value(old) };
            }
        }
    }

    pub fn clear(&self) {
        let _guard = ebr::pin::<V>();
        for shard in self.shards.iter() {
            let t = shard.table();
            for slot in t.slots.iter() {
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

    #[inline]
    pub fn shard_count(&self) -> usize {
        self.shards.len()
    }

    #[inline]
    pub fn shard_slot_count(&self, shard: usize) -> usize {
        self.shards[shard].table().capacity()
    }

    pub fn peek_slot(&self, shard: usize, slot: usize) -> Option<(String, V)> {
        let s = &self.shards[shard];
        let t = s.table();
        if slot >= t.slots.len() {
            return None;
        }
        let _guard = ebr::pin::<V>();
        let p = t.slots[slot].load(Ordering::Acquire);
        if p.is_null() {
            return None;
        }
        let entry = unsafe { &*p };
        let vptr = entry.value.load(Ordering::Acquire);
        if vptr.is_null() {
            return None;
        }
        Some((entry.key.clone(), unsafe { (*vptr).0.clone() }))
    }
}

unsafe impl<V: Send + Sync> Sync for CustomMap<V> {}
unsafe impl<V: Send + Sync> Send for CustomMap<V> {}

#[cfg(test)]
mod tests {
    use super::CustomMap;
    use std::sync::Arc;

    #[test]
    fn concurrent_readers_survive_repeated_growth() {
        let map = Arc::new(CustomMap::with_capacity(1, 1));
        for i in 0..128 {
            map.insert(format!("stable-{i}"), i);
        }

        std::thread::scope(|scope| {
            for _ in 0..4 {
                let map = Arc::clone(&map);
                scope.spawn(move || {
                    for _ in 0..20_000 {
                        for i in 0..128 {
                            assert_eq!(map.get(&format!("stable-{i}")), Some(i));
                        }
                    }
                });
            }

            for writer in 0..4 {
                let map = Arc::clone(&map);
                scope.spawn(move || {
                    for i in 0..2_000 {
                        let key = format!("writer-{writer}-{i}");
                        assert!(map.insert(key, i));
                    }
                });
            }
        });

        assert_eq!(map.len(), 8_128);
        for writer in 0..4 {
            for i in 0..2_000 {
                assert_eq!(map.get(&format!("writer-{writer}-{i}")), Some(i));
            }
        }
    }
}
