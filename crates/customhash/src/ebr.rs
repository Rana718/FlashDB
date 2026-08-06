use std::any::TypeId;
use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicU64, Ordering, fence};

use crossbeam_utils::CachePadded;

use crate::{ValueBox, free_value, new_value};

const INACTIVE: u64 = 0;
const COLLECT_INTERVAL: usize = 512;
const VALUE_POOL_LIMIT: usize = 1024;

static GLOBAL_EPOCH: AtomicU64 = AtomicU64::new(1);
static PARTICIPANTS: AtomicPtr<Participant> = AtomicPtr::new(ptr::null_mut());
static ORPHANS: AtomicPtr<OrphanNode> = AtomicPtr::new(ptr::null_mut());

struct Participant {
    local: CachePadded<AtomicU64>,
    next: *mut Participant,
}

unsafe impl Sync for Participant {}
unsafe impl Send for Participant {}

struct Garbage {
    ptr: *mut u8,
    drop_fn: unsafe fn(*mut u8),
    type_id: TypeId,
    epoch: u64,
}

unsafe impl Send for Garbage {}

struct OrphanNode {
    garbage: Vec<Garbage>,
    next: *mut OrphanNode,
}

unsafe fn drop_value_box<V>(ptr: *mut u8) {
    unsafe { free_value(ptr as *mut ValueBox<V>) };
}

struct Local {
    participant: *const Participant,
    garbage: Vec<Garbage>,
    depth: usize,
    retires: usize,
    pool: Vec<(*mut u8, unsafe fn(*mut u8), TypeId)>,
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
    fn initialize(&mut self) {
        let p = Box::into_raw(Box::new(Participant {
            local: CachePadded::new(AtomicU64::new(INACTIVE)),
            next: ptr::null_mut(),
        }));
        loop {
            let head = PARTICIPANTS.load(Ordering::Acquire);
            unsafe { (*p).next = head };
            if PARTICIPANTS
                .compare_exchange_weak(head, p, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                break;
            }
        }
        self.participant = p;
        self.garbage = Vec::with_capacity(COLLECT_INTERVAL * 2);
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
            unsafe { &*self.participant }
                .local
                .store(INACTIVE, Ordering::Release);
        }
    }

    fn collect(&mut self) {
        if !ORPHANS.load(Ordering::Relaxed).is_null() {
            let mut p = ORPHANS.swap(ptr::null_mut(), Ordering::AcqRel);
            while !p.is_null() {
                let node = unsafe { Box::from_raw(p) };
                p = node.next;
                self.garbage.extend(node.garbage);
            }
        }

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
        let reclaimable = self.garbage.partition_point(|g| g.epoch + 2 <= safe);
        if reclaimable != 0 {
            for item in self.garbage.drain(..reclaimable) {
                if self.pool.len() < VALUE_POOL_LIMIT {
                    self.pool.push((item.ptr, item.drop_fn, item.type_id));
                } else {
                    unsafe { (item.drop_fn)(item.ptr) };
                }
            }
        }
    }

    #[inline(always)]
    fn retire_raw(&mut self, ptr: *mut u8, drop_fn: unsafe fn(*mut u8), type_id: TypeId) {
        let epoch = GLOBAL_EPOCH.load(Ordering::Relaxed);
        self.garbage.push(Garbage {
            ptr,
            drop_fn,
            type_id,
            epoch,
        });
        self.retires += 1;
        if self.retires % COLLECT_INTERVAL == 0 {
            self.collect();
        }
    }

    #[inline(always)]
    fn alloc_value<V: 'static>(&mut self, value: V) -> *mut ValueBox<V> {
        let type_id = TypeId::of::<V>();
        if let Some(index) = self.pool.iter().rposition(|entry| entry.2 == type_id) {
            let (ptr, _, _) = self.pool.swap_remove(index);
            let ptr = ptr as *mut ValueBox<V>;
            unsafe {
                ptr::drop_in_place(&mut (*ptr).0);
                ptr::write(&mut (*ptr).0, value);
            }
            ptr
        } else {
            new_value(value)
        }
    }

    #[inline(always)]
    fn replace<V: 'static>(&mut self, slot: &AtomicPtr<ValueBox<V>>, value: V) -> bool {
        self.ensure_init();
        let new_ptr = self.alloc_value(value);
        let old = slot.swap(new_ptr, Ordering::AcqRel);
        if old.is_null() {
            return true;
        }
        self.retire_raw(old as *mut u8, drop_value_box::<V>, TypeId::of::<V>());
        false
    }
}

thread_local! {
    static LOCAL: UnsafeCell<Local> = UnsafeCell::new(Local::uninit());
}

pub struct Guard {
    _p: PhantomData<*const ()>,
}

impl Drop for Guard {
    #[inline(always)]
    fn drop(&mut self) {
        LOCAL.with(|c| unsafe { &mut *c.get() }.unpin());
    }
}

#[inline(always)]
pub fn pin<V>() -> Guard {
    LOCAL.with(|c| unsafe { &mut *c.get() }.pin());
    Guard { _p: PhantomData }
}

#[inline(always)]
pub unsafe fn retire_value<V: 'static>(ptr: *mut ValueBox<V>) {
    if ptr.is_null() {
        return;
    }
    LOCAL.with(|c| {
        let l = unsafe { &mut *c.get() };
        l.ensure_init();
        l.retire_raw(ptr as *mut u8, drop_value_box::<V>, TypeId::of::<V>());
    });
}

#[inline(always)]
pub fn replace_value<V: 'static>(slot: &AtomicPtr<ValueBox<V>>, value: V) -> bool {
    LOCAL.with(|c| unsafe { &mut *c.get() }.replace(slot, value))
}

#[inline(always)]
pub fn read_clone<V: Clone>(slot: &AtomicPtr<ValueBox<V>>) -> Option<V> {
    LOCAL.with(|c| {
        let l = unsafe { &mut *c.get() };
        l.pin();
        let ptr = slot.load(Ordering::Acquire);
        let r = if ptr.is_null() {
            None
        } else {
            Some(unsafe { (*ptr).0.clone() })
        };
        l.unpin();
        r
    })
}

#[inline(always)]
pub fn with_pin<R>(f: impl FnOnce(&()) -> R) -> R {
    LOCAL.with(|c| {
        let l = unsafe { &mut *c.get() };
        l.pin();
        let r = f(&());
        l.unpin();
        r
    })
}
