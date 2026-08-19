use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicU64, Ordering, fence};

use crossbeam_utils::CachePadded;

const INACTIVE: u64 = 0;
const COLLECT_INTERVAL: usize = 512;

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
    epoch: u64,
}

unsafe impl Send for Garbage {}

struct OrphanNode {
    garbage: Vec<Garbage>,
    next: *mut OrphanNode,
}

struct Local {
    participant: *const Participant,
    garbage: Vec<Garbage>,
    depth: usize,
    retires: usize,
    collect_on_unpin: bool,
    initialized: bool,
}

impl Local {
    const fn uninit() -> Self {
        Local {
            participant: ptr::null(),
            garbage: Vec::new(),
            depth: 0,
            retires: 0,
            collect_on_unpin: false,
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
            if self.collect_on_unpin {
                self.collect();
                self.collect();
                self.collect_on_unpin = !self.garbage.is_empty();
            }
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
                unsafe { (item.drop_fn)(item.ptr) };
            }
        }
    }

    #[inline(always)]
    fn retire_raw(&mut self, ptr: *mut u8, drop_fn: unsafe fn(*mut u8)) {
        let epoch = GLOBAL_EPOCH.load(Ordering::Relaxed);
        self.garbage.push(Garbage {
            ptr,
            drop_fn,
            epoch,
        });
        self.retires += 1;
        if self.retires % COLLECT_INTERVAL == 0 {
            self.collect();
        }
    }
}

pub fn force_collect() {
    LOCAL.with(|c| {
        let l = unsafe { &mut *c.get() };
        if !l.initialized {
            return;
        }
        l.collect();
        l.collect();
        l.collect();
    });
}

/// Demand-driven quiescence used by destructive commands.  It never frees an
/// object while a reader is pinned; it simply gives concurrent readers a short
/// chance to leave their critical section before collecting retired storage.
pub fn force_collect_quiescent() {
    for _ in 0..64 {
        force_collect();
        std::thread::yield_now();
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
pub fn pin() -> Guard {
    LOCAL.with(|c| unsafe { &mut *c.get() }.pin());
    Guard { _p: PhantomData }
}

/// Retire an explicitly allocated object. The callback must destroy the
/// object and release its allocation exactly once after the EBR grace period.
#[inline]
pub unsafe fn retire_raw(ptr: *mut u8, drop_fn: unsafe fn(*mut u8)) {
    if ptr.is_null() {
        return;
    }
    LOCAL.with(|c| {
        let l = unsafe { &mut *c.get() };
        l.ensure_init();
        l.retire_raw(ptr, drop_fn);
        l.collect_on_unpin = true;
    });
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
