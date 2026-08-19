mod ebr;

pub use ebr::{force_collect, force_collect_quiescent};

use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::ops::Deref;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicU32, AtomicUsize, Ordering};

use crossbeam_utils::CachePadded;
use foldhash::fast::RandomState;
use std::hash::BuildHasher;

const INLINE_CAP: usize = 15;

#[repr(C)]
struct CompactKey {
    data: [u8; INLINE_CAP],
    tag: u8,
}

impl CompactKey {
    #[inline]
    fn store_heap_len(data: &mut [u8; INLINE_CAP], len: usize) {
        let lb = (len as u64).to_ne_bytes();
        data[8..15].copy_from_slice(&lb[..7]);
    }

    #[inline(always)]
    fn read_heap_len(data: &[u8; INLINE_CAP]) -> usize {
        let mut lb = [0u8; 8];
        lb[..7].copy_from_slice(&data[8..15]);
        u64::from_ne_bytes(lb) as usize
    }

    fn from_string(s: String) -> Self {
        if s.len() <= INLINE_CAP {
            let mut data = [0u8; INLINE_CAP];
            data[..s.len()].copy_from_slice(s.as_bytes());
            Self { data, tag: s.len() as u8 }
        } else {
            let ptr = Box::into_raw(s.into_boxed_str());
            let mut data = [0u8; INLINE_CAP];
            let addr = (ptr as *const u8 as usize).to_ne_bytes();
            data[..8].copy_from_slice(&addr);
            Self::store_heap_len(&mut data, unsafe { &*ptr }.len());
            Self { data, tag: 0xFF }
        }
    }

    #[inline(always)]
    fn as_str(&self) -> &str {
        if self.tag != 0xFF {
            unsafe { std::str::from_utf8_unchecked(&self.data[..self.tag as usize]) }
        } else {
            let ptr_val = usize::from_ne_bytes(self.data[..8].try_into().unwrap());
            let len_val = Self::read_heap_len(&self.data);
            unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr_val as *const u8, len_val)) }
        }
    }
}

impl Drop for CompactKey {
    fn drop(&mut self) {
        if self.tag == 0xFF {
            let ptr_val = usize::from_ne_bytes(self.data[..8].try_into().unwrap());
            let len_val = Self::read_heap_len(&self.data);
            unsafe {
                drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr_val as *mut u8, len_val) as *mut str));
            }
        }
    }
}

impl PartialEq<&str> for CompactKey {
    #[inline(always)]
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl std::fmt::Display for CompactKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

struct Entry<V> {
    hash: u64,
    key: CompactKey,
    mlock: AtomicU8,
    seq: AtomicU32,
    occupied: AtomicBool,
    value: UnsafeCell<std::mem::MaybeUninit<V>>,
}

impl<V> Drop for Entry<V> {
    fn drop(&mut self) {
        if *self.occupied.get_mut() {
            unsafe { self.value.get_mut().assume_init_drop() };
        }
    }
}

unsafe impl<V: Send> Send for Entry<V> {}
unsafe impl<V: Send + Sync> Sync for Entry<V> {}

const LOAD_NUM: usize = 9;
const LOAD_DEN: usize = 10;
const DEFAULT_SHARD_CAPACITY: usize = 32_768;
const INITIAL_SHARD_CAPACITY: usize = 64;
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
        let _guard = ebr::pin();
        if let Some(existing) = self.find(&key, hash) {
            if existing.occupied.load(Ordering::Acquire) {
                spin_lock(&existing.mlock);
                existing.seq.fetch_add(1, Ordering::Release);
                unsafe { (*existing.value.get()).assume_init_drop() };
                unsafe { (*existing.value.get()).write(value) };
                existing.seq.fetch_add(1, Ordering::Release);
                spin_unlock(&existing.mlock);
                return false;
            }
            spin_lock(&existing.mlock);
            existing.seq.fetch_add(1, Ordering::Release);
            unsafe { (*existing.value.get()).write(value) };
            existing.occupied.store(true, Ordering::Release);
            existing.seq.fetch_add(1, Ordering::Release);
            spin_unlock(&existing.mlock);
            return true;
        }
        self.insert_new(key, value, hash)
    }

    fn insert_new(&self, key: String, value: V, hash: u64) -> bool {
        let entry: *mut Entry<V> = Box::into_raw(Box::new(Entry {
            hash,
            key: CompactKey::from_string(key),
            mlock: AtomicU8::new(0),
            seq: AtomicU32::new(0),
            occupied: AtomicBool::new(true),
            value: UnsafeCell::new(std::mem::MaybeUninit::new(value)),
        }));
        let key_ref: &str = unsafe { (*entry).key.as_str() };

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
                    if reserved {
                        self.len.fetch_sub(1, Ordering::Relaxed);
                    }
                    let was_occupied = e.occupied.load(Ordering::Acquire);
                    spin_lock(&e.mlock);
                    e.seq.fetch_add(1, Ordering::Release);
                    if e.occupied.load(Ordering::Relaxed) {
                        unsafe { (*e.value.get()).assume_init_drop() };
                    }
                    let moved_value = unsafe { (*(*entry).value.get()).assume_init_read() };
                    unsafe { (*e.value.get()).write(moved_value) };
                    e.occupied.store(true, Ordering::Release);
                    e.seq.fetch_add(1, Ordering::Release);
                    spin_unlock(&e.mlock);
                    unsafe {
                        (*entry).occupied.store(false, Ordering::Relaxed);
                        drop(Box::from_raw(entry));
                    }
                    return !was_occupied;
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

        let mut live_count: usize = 0;
        for slot in old_table.slots.iter() {
            let p = slot.load(Ordering::Acquire);
            if p.is_null() {
                continue;
            }
            let e = unsafe { &*p };
            if !e.occupied.load(Ordering::Acquire) {
                unsafe { ebr::retire_box(p) };
                continue;
            }

            live_count += 1;
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

        self.len.store(live_count, Ordering::Relaxed);

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
                    unsafe { drop(Box::from_raw(p)) };
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
    ptr: *const V,
    _guard: ebr::Guard,
    _map: PhantomData<&'a CustomMap<V>>,
}

impl<V> Deref for ValueRef<'_, V> {
    type Target = V;
    #[inline(always)]
    fn deref(&self) -> &V {
        unsafe { &*self.ptr }
    }
}

pub struct CustomMap<V> {
    shards: Box<[Shard<V>]>,
    shift: u32,
    shard_mask: usize,
    hasher: RandomState,
    key_count: CachePadded<AtomicUsize>,
    max_keys: usize,
}

impl<V: Clone + Send + Sync + 'static> CustomMap<V> {
    pub fn with_shards(n: usize) -> Self {
        Self::build(n.next_power_of_two().max(1), DEFAULT_SHARD_CAPACITY, usize::MAX)
    }

    pub fn with_capacity(shard_count: usize, expected_keys: usize) -> Self {
        let n = shard_count.next_power_of_two().max(1);
        // `expected_keys` is a capacity limit, not a reason to reserve the
        // complete table up front. Lazy growth keeps idle RSS small and still
        // uses the same lock-free shard growth path under load.
        let _ = expected_keys;
        Self::build(n, INITIAL_SHARD_CAPACITY, expected_keys)
    }

    fn build(n: usize, per_shard: usize, max_keys: usize) -> Self {
        CustomMap {
            shift: (u64::BITS - n.trailing_zeros()).min(63),
            shard_mask: n - 1,
            shards: (0..n)
                .map(|_| Shard::new(per_shard))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            hasher: RandomState::default(),
            key_count: CachePadded::new(AtomicUsize::new(0)),
            max_keys,
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
            if !entry.occupied.load(Ordering::Acquire) {
                return None;
            }
            Some(f(unsafe { (*entry.value.get()).assume_init_ref() }))
        })
    }

    #[inline]
    pub fn contains_key(&self, key: &str) -> bool {
        let (h, idx) = self.locate(key);
        ebr::with_pin(|_| {
            unsafe { self.shards.get_unchecked(idx) }
                .find(key, h)
                .is_some_and(|e| e.occupied.load(Ordering::Acquire))
        })
    }

    #[inline]
    pub fn get(&self, key: &str) -> Option<V> {
        let (h, idx) = self.locate(key);
        ebr::with_pin(|_| {
            let e = unsafe { self.shards.get_unchecked(idx) }.find(key, h)?;
            if !e.occupied.load(Ordering::Acquire) {
                return None;
            }
            Some(unsafe { (*e.value.get()).assume_init_ref().clone() })
        })
    }

    #[inline]
    pub fn get_ref(&self, key: &str) -> Option<ValueRef<'_, V>> {
        let (h, idx) = self.locate(key);
        let guard = ebr::pin();
        let e = unsafe { self.shards.get_unchecked(idx) }.find(key, h)?;
        if !e.occupied.load(Ordering::Acquire) {
            return None;
        }
        Some(ValueRef {
            ptr: unsafe { (*e.value.get()).as_ptr() },
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
        if self.key_count.load(Ordering::Relaxed) >= self.max_keys {
            let (h, idx) = self.locate(&key);
            let exists = ebr::with_pin(|_| {
                unsafe { self.shards.get_unchecked(idx) }
                    .find(&key, h)
                    .is_some_and(|e| e.occupied.load(Ordering::Acquire))
            });
            if !exists {
                return Err(Full);
            }
        }
        Ok(self.insert(key, value))
    }

    #[inline]
    pub fn set(&self, key: &str, value: V, key_owned: impl FnOnce() -> String) -> bool {
        let (h, idx) = self.locate(key);
        let _guard = ebr::pin();
        let shard = unsafe { self.shards.get_unchecked(idx) };
        if let Some(existing) = shard.find(key, h) {
            if existing.occupied.load(Ordering::Acquire) {
                spin_lock(&existing.mlock);
                existing.seq.fetch_add(1, Ordering::Release);
                unsafe { (*existing.value.get()).assume_init_drop() };
                unsafe { (*existing.value.get()).write(value) };
                existing.seq.fetch_add(1, Ordering::Release);
                spin_unlock(&existing.mlock);
                return false;
            }
            spin_lock(&existing.mlock);
            existing.seq.fetch_add(1, Ordering::Release);
            unsafe { (*existing.value.get()).write(value) };
            existing.occupied.store(true, Ordering::Release);
            existing.seq.fetch_add(1, Ordering::Release);
            spin_unlock(&existing.mlock);
            self.key_count.fetch_add(1, Ordering::Relaxed);
            return true;
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
        if self.key_count.load(Ordering::Relaxed) >= self.max_keys {
            let (h, idx) = self.locate(key);
            let exists = ebr::with_pin(|_| {
                unsafe { self.shards.get_unchecked(idx) }
                    .find(key, h)
                    .is_some_and(|e| e.occupied.load(Ordering::Acquire))
            });
            if !exists {
                return Err(Full);
            }
        }
        Ok(self.set(key, value, key_owned))
    }

    #[inline]
    pub fn remove(&self, key: &str) -> Option<V> {
        let (h, idx) = self.locate(key);
        let _guard = ebr::pin();
        let entry = unsafe { self.shards.get_unchecked(idx) }.find(key, h)?;
        if !entry.occupied.load(Ordering::Acquire) {
            return None;
        }
        spin_lock(&entry.mlock);
        if !entry.occupied.load(Ordering::Relaxed) {
            spin_unlock(&entry.mlock);
            return None;
        }
        entry.seq.fetch_add(1, Ordering::Release);
        let value = unsafe { (*entry.value.get()).assume_init_ref().clone() };
        unsafe { (*entry.value.get()).assume_init_drop() };
        entry.occupied.store(false, Ordering::Release);
        entry.seq.fetch_add(1, Ordering::Release);
        spin_unlock(&entry.mlock);
        self.key_count.fetch_sub(1, Ordering::Relaxed);
        Some(value)
    }

    #[inline]
    pub fn remove_no_clone(&self, key: &str) -> bool {
        let (h, idx) = self.locate(key);
        let _guard = ebr::pin();
        let entry = match unsafe { self.shards.get_unchecked(idx) }.find(key, h) {
            Some(e) => e,
            None => return false,
        };
        if !entry.occupied.load(Ordering::Acquire) {
            return false;
        }
        spin_lock(&entry.mlock);
        if !entry.occupied.load(Ordering::Relaxed) {
            spin_unlock(&entry.mlock);
            return false;
        }
        entry.seq.fetch_add(1, Ordering::Release);
        unsafe { (*entry.value.get()).assume_init_drop() };
        entry.occupied.store(false, Ordering::Release);
        entry.seq.fetch_add(1, Ordering::Release);
        spin_unlock(&entry.mlock);
        self.key_count.fetch_sub(1, Ordering::Relaxed);
        true
    }

    #[inline]
    pub fn remove_with<R>(&self, key: &str, f: impl FnOnce(&V) -> R) -> Option<R> {
        let (h, idx) = self.locate(key);
        let _guard = ebr::pin();
        let entry = unsafe { self.shards.get_unchecked(idx) }.find(key, h)?;
        if !entry.occupied.load(Ordering::Acquire) {
            return None;
        }
        spin_lock(&entry.mlock);
        if !entry.occupied.load(Ordering::Relaxed) {
            spin_unlock(&entry.mlock);
            return None;
        }
        let result = f(unsafe { (*entry.value.get()).assume_init_ref() });
        entry.seq.fetch_add(1, Ordering::Release);
        unsafe { (*entry.value.get()).assume_init_drop() };
        entry.occupied.store(false, Ordering::Release);
        entry.seq.fetch_add(1, Ordering::Release);
        spin_unlock(&entry.mlock);
        self.key_count.fetch_sub(1, Ordering::Relaxed);
        Some(result)
    }

    #[inline]
    pub fn update<R>(&self, key: &str, mut f: impl FnMut(&V) -> (V, R)) -> Option<R> {
        let (h, idx) = self.locate(key);
        let _guard = ebr::pin();
        let entry = unsafe { self.shards.get_unchecked(idx) }.find(key, h)?;

        spin_lock(&entry.mlock);
        if !entry.occupied.load(Ordering::Relaxed) {
            spin_unlock(&entry.mlock);
            return None;
        }
        let (new_val, result) = f(unsafe { (*entry.value.get()).assume_init_ref() });
        entry.seq.fetch_add(1, Ordering::Release);
        unsafe { (*entry.value.get()).assume_init_drop() };
        unsafe { (*entry.value.get()).write(new_val) };
        entry.seq.fetch_add(1, Ordering::Release);
        spin_unlock(&entry.mlock);
        Some(result)
    }

    #[inline]
    pub fn try_update<R>(&self, key: &str, mut f: impl FnMut(&V) -> Option<(V, R)>) -> Option<R> {
        let (h, idx) = self.locate(key);
        let _guard = ebr::pin();
        let entry = unsafe { self.shards.get_unchecked(idx) }.find(key, h)?;

        spin_lock(&entry.mlock);
        if !entry.occupied.load(Ordering::Relaxed) {
            spin_unlock(&entry.mlock);
            return None;
        }
        let result = f(unsafe { (*entry.value.get()).assume_init_ref() });
        match result {
            Some((new_val, r)) => {
                entry.seq.fetch_add(1, Ordering::Release);
                unsafe { (*entry.value.get()).assume_init_drop() };
                unsafe { (*entry.value.get()).write(new_val) };
                entry.seq.fetch_add(1, Ordering::Release);
                spin_unlock(&entry.mlock);
                Some(r)
            }
            None => {
                spin_unlock(&entry.mlock);
                None
            }
        }
    }

    #[inline]
    pub fn update_with<R>(
        &self,
        key: &str,
        mut f: impl FnMut(&mut V) -> R,
    ) -> Option<R> {
        let (h, idx) = self.locate(key);
        let _guard = ebr::pin();
        let entry = unsafe { self.shards.get_unchecked(idx) }.find(key, h)?;

        spin_lock(&entry.mlock);
        if !entry.occupied.load(Ordering::Relaxed) {
            spin_unlock(&entry.mlock);
            return None;
        }
        entry.seq.fetch_add(1, Ordering::Release);
        let result = f(unsafe { (*entry.value.get()).assume_init_mut() });
        entry.seq.fetch_add(1, Ordering::Release);
        spin_unlock(&entry.mlock);
        Some(result)
    }

    #[inline]
    pub fn get_locked<R>(
        &self,
        key: &str,
        f: impl FnOnce(&V) -> R,
    ) -> Option<R> {
        let (h, idx) = self.locate(key);
        let _guard = ebr::pin();
        let entry = unsafe { self.shards.get_unchecked(idx) }.find(key, h)?;

        spin_lock(&entry.mlock);
        if !entry.occupied.load(Ordering::Relaxed) {
            spin_unlock(&entry.mlock);
            return None;
        }
        let result = f(unsafe { (*entry.value.get()).assume_init_ref() });
        spin_unlock(&entry.mlock);
        Some(result)
    }

    #[inline]
    pub fn read_consistent<R>(
        &self,
        key: &str,
        f: impl Fn(&V) -> R,
    ) -> Option<R> {
        let (h, idx) = self.locate(key);
        let _guard = ebr::pin();
        let entry = unsafe { self.shards.get_unchecked(idx) }.find(key, h)?;

        loop {
            let seq1 = entry.seq.load(Ordering::Acquire);
            if seq1 & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            if !entry.occupied.load(Ordering::Acquire) {
                return None;
            }
            let result = f(unsafe { (*entry.value.get()).assume_init_ref() });
            std::sync::atomic::fence(Ordering::Acquire);
            let seq2 = entry.seq.load(Ordering::Acquire);
            if seq1 == seq2 {
                return Some(result);
            }
        }
    }

    #[inline]
    pub fn insert_if_absent(&self, key: String, value: V) -> bool {
        let (h, idx) = self.locate(&key);
        let _guard = ebr::pin();
        let shard = unsafe { self.shards.get_unchecked(idx) };
        if let Some(entry) = shard.find(&key, h) {
            if entry.occupied.load(Ordering::Acquire) {
                return false;
            }
            spin_lock(&entry.mlock);
            if entry.occupied.load(Ordering::Relaxed) {
                spin_unlock(&entry.mlock);
                return false;
            }
            entry.seq.fetch_add(1, Ordering::Release);
            unsafe { (*entry.value.get()).write(value) };
            entry.occupied.store(true, Ordering::Release);
            entry.seq.fetch_add(1, Ordering::Release);
            spin_unlock(&entry.mlock);
            self.key_count.fetch_add(1, Ordering::Relaxed);
            return true;
        }
        let is_new = shard.insert_new(key, value, h);
        if is_new {
            self.key_count.fetch_add(1, Ordering::Relaxed);
        }
        is_new
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
        for shard in self.shards.iter() {
            let _guard = ebr::pin();
            let t = shard.table();
            for slot in t.slots.iter() {
                let p = slot.load(Ordering::Acquire);
                if p.is_null() {
                    continue;
                }
                let e = unsafe { &*p };
                if e.occupied.load(Ordering::Acquire) {
                    f(e.key.as_str(), unsafe { (*e.value.get()).assume_init_ref() });
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
        for shard in self.shards.iter() {
            let _guard = ebr::pin();
            let t = shard.table();
            for slot in t.slots.iter() {
                let p = slot.load(Ordering::Acquire);
                if p.is_null() {
                    continue;
                }
                let entry = unsafe { &*p };
                if !entry.occupied.load(Ordering::Acquire) {
                    continue;
                }
                if !f(entry.key.as_str(), unsafe { (*entry.value.get()).assume_init_ref() }) {
                    spin_lock(&entry.mlock);
                    if entry.occupied.load(Ordering::Relaxed) {
                        entry.seq.fetch_add(1, Ordering::Release);
                        unsafe { (*entry.value.get()).assume_init_drop() };
                        entry.occupied.store(false, Ordering::Release);
                        entry.seq.fetch_add(1, Ordering::Release);
                        self.key_count.fetch_sub(1, Ordering::Relaxed);
                    }
                    spin_unlock(&entry.mlock);
                }
            }
        }
    }

    pub fn retain_shard(&self, shard_idx: usize, mut f: impl FnMut(&str, &V) -> bool) {
        let _ = self.retain_shard_range(shard_idx, 0, usize::MAX, |key, value| f(key, value));
    }

    pub fn retain_shard_range(
        &self,
        shard_idx: usize,
        start_slot: usize,
        max_slots: usize,
        mut f: impl FnMut(&str, &V) -> bool,
    ) -> Option<(usize, usize)> {
        let Some(shard) = self.shards.get(shard_idx) else {
            return None;
        };
        let _guard = ebr::pin();
        let t = shard.table();
        let capacity = t.capacity();
        let start = start_slot.min(capacity);
        let end = start.saturating_add(max_slots).min(capacity);
        for slot in &t.slots[start..end] {
            let p = slot.load(Ordering::Acquire);
            if p.is_null() {
                continue;
            }
            let entry = unsafe { &*p };
            if !entry.occupied.load(Ordering::Acquire) {
                continue;
            }
            if f(entry.key.as_str(), unsafe { (*entry.value.get()).assume_init_ref() }) {
                continue;
            }
            spin_lock(&entry.mlock);
            if entry.occupied.load(Ordering::Relaxed) {
                entry.seq.fetch_add(1, Ordering::Release);
                unsafe { (*entry.value.get()).assume_init_drop() };
                entry.occupied.store(false, Ordering::Release);
                entry.seq.fetch_add(1, Ordering::Release);
                self.key_count.fetch_sub(1, Ordering::Relaxed);
            }
            spin_unlock(&entry.mlock);
        }
        Some((end, capacity))
    }

    pub fn compact_shard(&self, shard_idx: usize) {
        let Some(shard) = self.shards.get(shard_idx) else {
            return;
        };
        let _lock = shard.grow_lock.lock().unwrap_or_else(|e| e.into_inner());
        while shard
            .insert_gate
            .compare_exchange_weak(0, GROWING, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            std::hint::spin_loop();
        }

        let old_ptr = shard.table.load(Ordering::Acquire);
        let old_table = unsafe { &*old_ptr };

        let mut live_count: usize = 0;
        for slot in old_table.slots.iter() {
            let p = slot.load(Ordering::Acquire);
            if !p.is_null() && unsafe { &*p }.occupied.load(Ordering::Acquire) {
                live_count += 1;
            }
        }

        let new_cap = ((live_count * LOAD_DEN / LOAD_NUM) + 1).next_power_of_two().max(8);
        if new_cap >= old_table.capacity() {
            shard.insert_gate.store(0, Ordering::Release);
            return;
        }

        let new_table = Box::new(SlotTable::<V>::new(new_cap));
        for slot in old_table.slots.iter() {
            let p = slot.load(Ordering::Acquire);
            if p.is_null() {
                continue;
            }
            let e = unsafe { &*p };
            if !e.occupied.load(Ordering::Acquire) {
                unsafe { ebr::retire_box(p) };
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

        shard.len.store(live_count, Ordering::Relaxed);
        let new_ptr = Box::into_raw(new_table);
        shard.table.store(new_ptr, Ordering::Release);
        unsafe { ebr::retire_box(old_ptr) };
        shard.insert_gate.store(0, Ordering::Release);
    }

    pub fn clear(&self) {
        for shard in self.shards.iter() {
            let _lock = shard.grow_lock.lock().unwrap_or_else(|e| e.into_inner());
            while shard
                .insert_gate
                .compare_exchange_weak(0, GROWING, Ordering::AcqRel, Ordering::Relaxed)
                .is_err()
            {
                std::hint::spin_loop();
            }
            let new_table = Box::into_raw(Box::new(SlotTable::<V>::new(INITIAL_SHARD_CAPACITY)));
            let old_ptr = shard.table.swap(new_table, Ordering::AcqRel);
            shard.len.store(0, Ordering::Relaxed);
            if !old_ptr.is_null() {
                let old_table = unsafe { &*old_ptr };
                for slot in old_table.slots.iter() {
                    let p = slot.load(Ordering::Relaxed);
                    if !p.is_null() {
                        unsafe { ebr::retire_box(p) };
                    }
                }
                unsafe { ebr::retire_box(old_ptr) };
            }
            shard.insert_gate.store(0, Ordering::Release);
        }
        self.key_count.store(0, Ordering::Release);
        force_collect();
    }

    #[inline]
    pub fn shard_count(&self) -> usize {
        self.shards.len()
    }

    #[inline]
    pub fn shard_slot_count(&self, shard: usize) -> usize {
        let _guard = ebr::pin();
        self.shards[shard].table().capacity()
    }

    pub fn shard_layout_matches(&self, capacities: &[usize]) -> bool {
        if capacities.len() != self.shards.len() {
            return false;
        }
        let _guard = ebr::pin();
        self.shards
            .iter()
            .zip(capacities)
            .all(|(shard, &capacity)| shard.table().capacity() == capacity)
    }

    pub fn peek_slot(&self, shard: usize, slot: usize) -> Option<(String, V)> {
        let s = &self.shards[shard];
        let t = s.table();
        if slot >= t.slots.len() {
            return None;
        }
        let _guard = ebr::pin();
        let p = t.slots[slot].load(Ordering::Acquire);
        if p.is_null() {
            return None;
        }
        let entry = unsafe { &*p };
        if !entry.occupied.load(Ordering::Acquire) {
            return None;
        }
        Some((entry.key.as_str().to_owned(), unsafe { (*entry.value.get()).assume_init_ref().clone() }))
    }

    pub fn peek_slot_with<R>(&self, shard: usize, slot: usize, f: impl FnOnce(&str, &V) -> R) -> Option<R> {
        let s = &self.shards[shard];
        let t = s.table();
        if slot >= t.slots.len() {
            return None;
        }
        let _guard = ebr::pin();
        let p = t.slots[slot].load(Ordering::Acquire);
        if p.is_null() {
            return None;
        }
        let entry = unsafe { &*p };
        if !entry.occupied.load(Ordering::Acquire) {
            return None;
        }
        Some(f(entry.key.as_str(), unsafe { (*entry.value.get()).assume_init_ref() }))
    }
}

unsafe impl<V: Send + Sync> Sync for CustomMap<V> {}
unsafe impl<V: Send + Sync> Send for CustomMap<V> {}

#[inline(always)]
fn spin_lock(lock: &AtomicU8) {
    if lock.swap(1, Ordering::Acquire) == 0 {
        return;
    }
    spin_lock_slow(lock);
}

#[cold]
fn spin_lock_slow(lock: &AtomicU8) {
    loop {
        while lock.load(Ordering::Relaxed) != 0 {
            std::hint::spin_loop();
        }
        if lock.swap(1, Ordering::Acquire) == 0 {
            return;
        }
    }
}

#[inline(always)]
fn spin_unlock(lock: &AtomicU8) {
    lock.store(0, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::CustomMap;
    use std::collections::HashSet;
    use std::sync::Arc;

    #[test]
    fn bounded_retain_covers_each_slot_once() {
        let map = CustomMap::with_capacity(1, 16);
        for i in 0..16 {
            map.insert(format!("key-{i}"), i);
        }

        let mut cursor = 0usize;
        let mut seen = HashSet::new();
        loop {
            let (next, capacity) = map
                .retain_shard_range(0, cursor, 17, |key, value| {
                    assert!(seen.insert(key.to_owned()));
                    value % 2 == 0
                })
                .unwrap();
            if next == capacity {
                break;
            }
            assert!(next > cursor);
            cursor = next;
        }

        assert_eq!(seen.len(), 16);
        assert_eq!(map.len(), 8);
        for i in 0..16 {
            assert_eq!(map.contains_key(&format!("key-{i}")), i % 2 == 0);
        }
    }

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

    #[test]
    fn concurrent_clear_read_write_is_safe() {
        let map = Arc::new(CustomMap::with_capacity(8, 100_000));
        std::thread::scope(|scope| {
            for worker in 0..4 {
                let map = Arc::clone(&map);
                scope.spawn(move || {
                    for i in 0..50_000 {
                        let key = format!("key-{worker}-{}", i % 2_000);
                        map.set(&key, i, || key.clone());
                        let _ = map.get(&key);
                    }
                });
            }

            let map = Arc::clone(&map);
            scope.spawn(move || {
                for _ in 0..100 {
                    map.clear();
                    std::thread::yield_now();
                }
            });
        });

        map.clear();
        assert_eq!(map.len(), 0);
    }
}
