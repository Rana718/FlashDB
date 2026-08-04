//! Epoch-based reclamation — generic over V.
//!
//! Optimized for throughput: uses #[thread_local] for zero-overhead TLS access,
//! value pooling to eliminate malloc on updates, and batched collection.

use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicU64, Ordering, fence};

use crossbeam_utils::CachePadded;

use crate::{ValueBox, free_value, new_value};

const INACTIVE: u64 = 0;
const COLLECT_INTERVAL: usize = 512;
const INITIAL_GARBAGE_CAPACITY: usize = COLLECT_INTERVAL * 2;
const VALUE_POOL_LIMIT: usize = 1024;

static GLOBAL_EPOCH: AtomicU64 = AtomicU64::new(1);
static PARTICIPANTS: AtomicPtr<Participant> = AtomicPtr::new(ptr::null_mut());

pub(crate) struct Participant {
    local: CachePadded<AtomicU64>,
    next: *mut Participant,
}

unsafe impl Sync for Participant {}
unsafe impl Send for Participant {}

/// Type-erased garbage item
struct Garbage {
    ptr: *mut u8,
    drop_fn: unsafe fn(*mut u8),
    epoch: u64,
}

unsafe impl Send for Garbage {}

unsafe fn drop_value_box<V>(ptr: *mut u8) {
    unsafe { free_value(ptr as *mut ValueBox<V>) };
}

static ORPHANS: AtomicPtr<OrphanNode> = AtomicPtr::new(ptr::null_mut());

struct OrphanNode {
    garbage: Vec<Garbage>,
    next: *mut OrphanNode,
}

#[allow(dead_code)]
fn push_orphans(garbage: Vec<Garbage>) {
    if garbage.is_empty() {
        return;
    }
    let node = Box::into_raw(Box::new(OrphanNode { garbage, next: ptr::null_mut() }));
    loop {
        let head = ORPHANS.load(Ordering::Acquire);
        unsafe { (*node).next = head };
        if ORPHANS
            .compare_exchange_weak(head, node, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return;
        }
    }
}

fn adopt_orphans(dst: &mut Vec<Garbage>) -> bool {
    if ORPHANS.load(Ordering::Relaxed).is_null() {
        return false;
    }
    let mut p = ORPHANS.swap(ptr::null_mut(), Ordering::AcqRel);
    if p.is_null() {
        return false;
    }
    while !p.is_null() {
        let node = unsafe { Box::from_raw(p) };
        p = node.next;
        dst.extend(node.garbage);
    }
    true
}

struct Local {
    participant: *const Participant,
    garbage: Vec<Garbage>,
    depth: usize,
    retires: usize,
    pool: Vec<(*mut u8, unsafe fn(*mut u8))>,
    initialized: bool,
}

impl Local {
    const fn uninit() -> Self {
        Local {
            participant: ptr::null(),
            garbage: Vec::new(),
            depth: 0,
            retires: 0,
            pool: Vec::new(),
            initialized: false,
        }
    }

    #[cold]
    #[inline(never)]
    fn initialize(&mut self) {
        let participant = Box::into_raw(Box::new(Participant {
            local: CachePadded::new(AtomicU64::new(INACTIVE)),
            next: ptr::null_mut(),
        }));
        loop {
            let head = PARTICIPANTS.load(Ordering::Acquire);
            unsafe { (*participant).next = head };
            if PARTICIPANTS
                .compare_exchange_weak(head, participant, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                break;
            }
        }
        self.participant = participant;
        self.garbage = Vec::with_capacity(INITIAL_GARBAGE_CAPACITY);
        self.pool = Vec::with_capacity(VALUE_POOL_LIMIT);
        self.initialized = true;
    }

    #[inline(always)]
    fn ensure_init(&mut self) {
        if !self.initialized {
            self.initialize();
        }
    }

    #[inline(always)]
    fn pin(&mut self) {
        self.ensure_init();
        self.depth += 1;
        if self.depth == 1 {
            let participant = unsafe { &*self.participant };
            loop {
                let e = GLOBAL_EPOCH.load(Ordering::Relaxed);
                participant.local.store(e, Ordering::Release);
                fence(Ordering::Acquire);
                if GLOBAL_EPOCH.load(Ordering::Acquire) == e {
                    break;
                }
                participant.local.store(INACTIVE, Ordering::Release);
            }
        }
    }

    #[inline(always)]
    fn unpin(&mut self) {
        self.depth -= 1;
        if self.depth == 0 {
            unsafe { &*self.participant }.local.store(INACTIVE, Ordering::Release);
        }
    }

    fn collect(&mut self) {
        let adopted = adopt_orphans(&mut self.garbage);

        let global = GLOBAL_EPOCH.load(Ordering::Acquire);

        let mut all_caught_up = true;
        let mut p = PARTICIPANTS.load(Ordering::Acquire);
        while !p.is_null() {
            let node = unsafe { &*p };
            let e = node.local.load(Ordering::Acquire);
            if e != INACTIVE && e < global {
                all_caught_up = false;
                break;
            }
            p = node.next;
        }
        if all_caught_up {
            let _ = GLOBAL_EPOCH.compare_exchange(
                global,
                global + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            );
        }

        let safe = GLOBAL_EPOCH.load(Ordering::Acquire);
        if adopted && !self.garbage.is_sorted_by_key(|item| item.epoch) {
            self.garbage.sort_unstable_by_key(|item| item.epoch);
        }
        let reclaimable = self.garbage.partition_point(|item| item.epoch + 2 <= safe);
        if reclaimable != 0 {
            for item in self.garbage.drain(..reclaimable) {
                if self.pool.len() < VALUE_POOL_LIMIT {
                    self.pool.push((item.ptr, item.drop_fn));
                } else {
                    unsafe { (item.drop_fn)(item.ptr) };
                }
            }
        }
    }

    #[inline(always)]
    fn retire_raw(&mut self, ptr: *mut u8, drop_fn: unsafe fn(*mut u8)) {
        let epoch = GLOBAL_EPOCH.load(Ordering::Relaxed);
        self.garbage.push(Garbage { ptr, drop_fn, epoch });
        self.retires += 1;
        if self.retires % COLLECT_INTERVAL == 0 {
            self.collect();
        }
    }

    #[inline(always)]
    fn alloc_value<V>(&mut self, value: V) -> *mut ValueBox<V> {
        if let Some((ptr, _drop_fn)) = self.pool.pop() {
            let p = ptr as *mut ValueBox<V>;
            unsafe {
                ptr::drop_in_place(&mut (*p).0);
                ptr::write(&mut (*p).0, value);
            }
            p
        } else {
            new_value(value)
        }
    }

    /// Allocate, swap, retire old — the SET hot path.
    #[inline(always)]
    fn replace<V>(&mut self, slot: &AtomicPtr<ValueBox<V>>, value: V) -> bool {
        self.ensure_init();
        let new_ptr = self.alloc_value(value);
        let old = slot.swap(new_ptr, Ordering::Release);
        if old.is_null() {
            return true;
        }
        self.retire_raw(old as *mut u8, drop_value_box::<V>);
        false
    }
}

// Use thread_local! — this is the standard approach.
// The key optimization: we access LOCAL only ONCE per operation.
thread_local! {
    static LOCAL: UnsafeCell<Local> = UnsafeCell::new(Local::uninit());
}

/// Epoch guard — keeps value pointers alive while held.
pub struct Guard {
    _not_send: PhantomData<*const ()>,
}

impl Drop for Guard {
    #[inline(always)]
    fn drop(&mut self) {
        LOCAL.with(|cell| unsafe { &mut *cell.get() }.unpin());
    }
}

#[inline(always)]
pub fn pin<V>() -> Guard {
    LOCAL.with(|cell| unsafe { &mut *cell.get() }.pin());
    Guard { _not_send: PhantomData }
}

/// Retire a value pointer for deferred freeing.
#[inline(always)]
pub unsafe fn retire_value<V>(ptr: *mut ValueBox<V>) {
    if ptr.is_null() {
        return;
    }
    LOCAL.with(|cell| {
        let l = unsafe { &mut *cell.get() };
        l.ensure_init();
        l.retire_raw(ptr as *mut u8, drop_value_box::<V>);
    });
}

/// SET hot path: alloc (pooled or fresh) + swap + retire old — single TLS access.
#[inline(always)]
pub fn replace_value<V>(slot: &AtomicPtr<ValueBox<V>>, value: V) -> bool {
    LOCAL.with(|cell| unsafe { &mut *cell.get() }.replace(slot, value))
}

/// GET hot path: pin + read + clone + unpin — single TLS access.
#[inline(always)]
pub fn read_clone<V: Clone>(slot: &AtomicPtr<ValueBox<V>>) -> Option<V> {
    LOCAL.with(|cell| {
        let l = unsafe { &mut *cell.get() };
        l.pin();
        let ptr = slot.load(Ordering::Acquire);
        let result = if ptr.is_null() {
            None
        } else {
            Some(unsafe { (*ptr).0.clone() })
        };
        l.unpin();
        result
    })
}

/// Pin, execute closure, unpin — single TLS access.
#[inline(always)]
pub fn with_pin<R>(f: impl FnOnce(&()) -> R) -> R {
    LOCAL.with(|cell| {
        let l = unsafe { &mut *cell.get() };
        l.pin();
        let result = f(&());
        l.unpin();
        result
    })
}
