use std::alloc::Layout;
use std::cell::UnsafeCell;
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering};

use crossbeam_utils::CachePadded;

use super::ebr;
use super::key::CompactKey;

pub(crate) const STATE_LOCK: u64 = 1;
pub(crate) const STATE_OCCUPIED: u64 = 1 << 1;
pub(crate) const STATE_SEQ_ONE: u64 = 1 << 2;

pub(crate) const LOAD_NUM: usize = 9;
pub(crate) const LOAD_DEN: usize = 10;
pub(crate) const DEFAULT_SHARD_CAPACITY: usize = 32_768;
// Keep idle shards tiny; tables grow lock-free as keys arrive.
pub(crate) const INITIAL_SHARD_CAPACITY: usize = 8;
pub(crate) const GROWING: usize = 1usize << (usize::BITS - 1);

pub(crate) struct Entry<V> {
    pub(crate) hash: u64,
    pub(crate) key: CompactKey,
    pub(crate) state: AtomicU64,
    pub(crate) value: UnsafeCell<std::mem::MaybeUninit<V>>,
}

pub(crate) unsafe fn drop_raw_entry<V>(ptr: *mut u8) {
    let entry = ptr.cast::<Entry<V>>();
    unsafe {
        std::ptr::drop_in_place(entry);
    }
    unsafe {
        rust_zmalloc::dealloc_raw(ptr, Layout::new::<Entry<V>>());
    }
}

pub(crate) fn alloc_entry<V>(entry: Entry<V>) -> *mut Entry<V> {
    let layout = Layout::new::<Entry<V>>();
    let ptr = unsafe { rust_zmalloc::alloc_raw(layout) }.cast::<Entry<V>>();
    if ptr.is_null() {
        std::alloc::handle_alloc_error(layout);
    }
    unsafe {
        ptr.write(entry);
    }
    ptr
}

impl<V> Drop for Entry<V> {
    fn drop(&mut self) {
        if (*self.state.get_mut() & STATE_OCCUPIED) != 0 {
            unsafe { self.value.get_mut().assume_init_drop() };
        }
    }
}

unsafe impl<V: Send> Send for Entry<V> {}
unsafe impl<V: Send + Sync> Sync for Entry<V> {}

#[inline]
pub(crate) fn state_seq(state: u64) -> u32 {
    (state >> 2) as u32
}

#[inline(always)]
pub(crate) fn state_occupied(state: &AtomicU64, order: Ordering) -> bool {
    state.load(order) & STATE_OCCUPIED != 0
}

#[inline(always)]
pub(crate) fn state_set_occupied(state: &AtomicU64, occupied: bool, order: Ordering) {
    if occupied {
        state.fetch_or(STATE_OCCUPIED, order);
    } else {
        state.fetch_and(!STATE_OCCUPIED, order);
    }
}

#[inline(always)]
pub(crate) fn state_seq_add(state: &AtomicU64, order: Ordering) {
    // Called while holding the lock, so no other writer can modify the seq bits.
    // Use fetch_add directly instead of CAS loop.
    state.fetch_add(STATE_SEQ_ONE, order);
}

#[inline(always)]
pub(crate) fn state_lock(state: &AtomicU64) {
    let mut current = state.load(Ordering::Relaxed);
    loop {
        if current & STATE_LOCK != 0 {
            state_lock_slow(state);
            return;
        }
        match state.compare_exchange_weak(
            current,
            current | STATE_LOCK,
            Ordering::Acquire,
            Ordering::Relaxed,
        ) {
            Ok(_) => return,
            Err(observed) => current = observed,
        }
    }
}

#[cold]
fn state_lock_slow(state: &AtomicU64) {
    loop {
        while state.load(Ordering::Relaxed) & STATE_LOCK != 0 {
            std::hint::spin_loop();
        }
        let current = state.load(Ordering::Relaxed);
        if current & STATE_LOCK == 0
            && state
                .compare_exchange_weak(
                    current,
                    current | STATE_LOCK,
                    Ordering::Acquire,
                    Ordering::Relaxed,
                )
                .is_ok()
        {
            return;
        }
    }
}

#[inline(always)]
pub(crate) fn state_unlock(state: &AtomicU64) {
    state.fetch_and(!STATE_LOCK, Ordering::Release);
}

pub(crate) struct SlotTable<V> {
    pub(crate) slots: *mut AtomicPtr<Entry<V>>,
    pub(crate) capacity: usize,
    pub(crate) mask: usize,
    pub(crate) threshold: usize,
}

unsafe impl<V: Send> Send for SlotTable<V> {}
unsafe impl<V: Send> Sync for SlotTable<V> {}

impl<V> Drop for SlotTable<V> {
    fn drop(&mut self) {
        if self.slots.is_null() {
            return;
        }
        let layout = Layout::array::<AtomicPtr<Entry<V>>>(self.capacity)
            .expect("slot array layout overflow");
        unsafe {
            rust_zmalloc::dealloc_raw(self.slots.cast(), layout);
        }
    }
}

pub(crate) unsafe fn drop_raw_table<V>(ptr: *mut u8) {
    let table = ptr.cast::<SlotTable<V>>();
    unsafe {
        std::ptr::drop_in_place(table);
    }
    unsafe {
        rust_zmalloc::dealloc_raw(ptr, Layout::new::<SlotTable<V>>());
    }
}

pub(crate) fn alloc_table<V>(table: SlotTable<V>) -> *mut SlotTable<V> {
    let layout = Layout::new::<SlotTable<V>>();
    let ptr = unsafe { rust_zmalloc::alloc_raw(layout) }.cast::<SlotTable<V>>();
    if ptr.is_null() {
        std::alloc::handle_alloc_error(layout);
    }
    unsafe {
        ptr.write(table);
    }
    ptr
}

impl<V> SlotTable<V> {
    pub(crate) fn new(cap: usize) -> Self {
        let cap = cap.next_power_of_two().max(8);
        let layout = Layout::array::<AtomicPtr<Entry<V>>>(cap).expect("slot array layout overflow");
        let slots = unsafe { rust_zmalloc::alloc_raw(layout) }.cast::<AtomicPtr<Entry<V>>>();
        if slots.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        for i in 0..cap {
            unsafe {
                slots.add(i).write(AtomicPtr::new(ptr::null_mut()));
            }
        }
        SlotTable {
            slots,
            capacity: cap,
            mask: cap - 1,
            threshold: cap * LOAD_NUM / LOAD_DEN,
        }
    }

    #[inline(always)]
    pub(crate) fn slots(&self) -> &[AtomicPtr<Entry<V>>] {
        unsafe { std::slice::from_raw_parts(self.slots, self.capacity) }
    }

    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }
}

#[repr(align(128))]
pub(crate) struct Shard<V> {
    pub(crate) table: AtomicPtr<SlotTable<V>>,
    pub(crate) len: CachePadded<AtomicUsize>,
    pub(crate) insert_gate: CachePadded<AtomicUsize>,
    pub(crate) grow_lock: std::sync::Mutex<()>,
}

pub(crate) struct InsertGuard<'a> {
    gate: &'a AtomicUsize,
}

impl Drop for InsertGuard<'_> {
    #[inline(always)]
    fn drop(&mut self) {
        self.gate.fetch_sub(1, Ordering::Release);
    }
}

impl<V: Clone + Send + Sync + 'static> Shard<V> {
    pub(crate) fn new(cap: usize) -> Self {
        let table = alloc_table(SlotTable::new(cap));
        Shard {
            table: AtomicPtr::new(table),
            len: CachePadded::new(AtomicUsize::new(0)),
            insert_gate: CachePadded::new(AtomicUsize::new(0)),
            grow_lock: std::sync::Mutex::new(()),
        }
    }

    #[inline(always)]
    pub(crate) fn table(&self) -> &SlotTable<V> {
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
    pub(crate) fn find(&self, key: &str, hash: u64) -> Option<&Entry<V>> {
        let t = self.table();
        let mut i = (hash as usize) & t.mask;
        loop {
            let p = unsafe { t.slots().get_unchecked(i) }.load(Ordering::Acquire);
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
    pub(crate) fn insert_hashed(&self, key: String, value: V, hash: u64) -> bool {
        let _guard = ebr::pin();
        if let Some(existing) = self.find(&key, hash) {
            if state_occupied(&existing.state, Ordering::Acquire) {
                state_lock(&existing.state);
                state_seq_add(&existing.state, Ordering::Release);
                unsafe { (*existing.value.get()).assume_init_drop() };
                unsafe { (*existing.value.get()).write(value) };
                state_seq_add(&existing.state, Ordering::Release);
                state_unlock(&existing.state);
                return false;
            }
            state_lock(&existing.state);
            state_seq_add(&existing.state, Ordering::Release);
            unsafe { (*existing.value.get()).write(value) };
            state_set_occupied(&existing.state, true, Ordering::Release);
            state_seq_add(&existing.state, Ordering::Release);
            state_unlock(&existing.state);
            return true;
        }
        self.insert_new(key, value, hash)
    }

    pub(crate) fn insert_new(&self, key: String, value: V, hash: u64) -> bool {
        let entry: *mut Entry<V> = alloc_entry(Entry {
            hash,
            key: CompactKey::from_string(key),
            state: AtomicU64::new(STATE_OCCUPIED),
            value: UnsafeCell::new(std::mem::MaybeUninit::new(value)),
        });
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
                let slot = unsafe { t.slots().get_unchecked(i) };
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
                    let was_occupied = state_occupied(&e.state, Ordering::Acquire);
                    state_lock(&e.state);
                    state_seq_add(&e.state, Ordering::Release);
                    if state_occupied(&e.state, Ordering::Relaxed) {
                        unsafe { (*e.value.get()).assume_init_drop() };
                    }
                    let moved_value = unsafe { (*(*entry).value.get()).assume_init_read() };
                    unsafe { (*e.value.get()).write(moved_value) };
                    state_set_occupied(&e.state, true, Ordering::Release);
                    state_seq_add(&e.state, Ordering::Release);
                    state_unlock(&e.state);
                    unsafe {
                        state_set_occupied(&(*entry).state, false, Ordering::Relaxed);
                        drop_raw_entry::<V>(entry.cast());
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
        for slot in old_table.slots().iter() {
            let p = slot.load(Ordering::Acquire);
            if p.is_null() {
                continue;
            }
            let e = unsafe { &*p };
            if !state_occupied(&e.state, Ordering::Acquire) {
                unsafe { ebr::retire_raw(p.cast(), drop_raw_entry::<V>) };
                continue;
            }

            live_count += 1;
            let mut i = (e.hash as usize) & new_table.mask;
            loop {
                let new_slot = unsafe { new_table.slots().get_unchecked(i) };
                if new_slot.load(Ordering::Relaxed).is_null() {
                    new_slot.store(p, Ordering::Relaxed);
                    break;
                }
                i = (i + 1) & new_table.mask;
            }
        }

        self.len.store(live_count, Ordering::Relaxed);

        let new_ptr = alloc_table(*new_table);
        self.table.store(new_ptr, Ordering::Release);
        unsafe { ebr::retire_raw(old_ptr.cast(), drop_raw_table::<V>) };
    }
}

impl<V> Drop for Shard<V> {
    fn drop(&mut self) {
        let t_ptr = self.table.load(Ordering::Relaxed);
        if !t_ptr.is_null() {
            let t = unsafe { &*t_ptr };
            for slot in t.slots().iter() {
                let p = slot.load(Ordering::Relaxed);
                if !p.is_null() {
                    unsafe { drop_raw_entry::<V>(p.cast()) };
                }
            }
            unsafe {
                drop_raw_table::<V>(t_ptr.cast());
            }
        }
    }
}

unsafe impl<V: Send> Sync for Shard<V> {}
unsafe impl<V: Send> Send for Shard<V> {}
