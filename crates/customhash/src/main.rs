//! CustomMap — fully lock-free sharded concurrent hash map.
//!
//! Read path  : wait-free. Plain `Acquire` loads walking a linear probe
//!              sequence. No lock, no fence, no atomic RMW, no retry loop.
//!              `contains_key` touches no reclamation machinery at all.
//!
//! Write path : lock-free. A new key is published with one `compare_exchange`
//!              on a slot; a value update is one `swap` on the entry's value
//!              pointer. No mutex anywhere, on any path.
//!
//! Why this replaces the previous seqlock design
//! ---------------------------------------------
//! The seqlock version was unsound, not merely slow:
//!
//!   * A reader cloning a `String` raced with a writer's `*v = value`, which
//!     drops and frees the old buffer. The reader could memcpy from freed
//!     memory. Re-checking the sequence number afterwards is too late — the
//!     use-after-free has already happened.
//!   * `HashTable::find` ran concurrently with `entry()`, which reallocates
//!     the bucket array on growth, so readers probed freed memory.
//!
//! A seqlock is only sound over trivially-copyable data read with volatile
//! loads. It cannot protect a `String` or a reallocating table. The old
//! benchmark never crashed only because it preallocated enough to avoid growth
//! and read with `contains_key` (keys are immutable after insert).
//!
//! The three invariants that make this version lock-free *and* sound
//! -----------------------------------------------------------------
//!   1. An `Entry` is immutable except for its value pointer, and is never
//!      freed while the map is alive (there is no `remove`). Readers may hold
//!      `&Entry` and read `hash`/`key` with no reclamation at all.
//!   2. Only the value pointer changes, and only by whole-pointer `swap`. The
//!      replaced buffer is retired through epoch-based reclamation, so a
//!      reader copying it is guaranteed it stays alive until it unpins.
//!   3. A slot is write-once: `null → entry`, never anything else. The slot
//!      array is never reallocated, so readers index it freely.
//!
//! Why concurrent inserts of the same key cannot duplicate it
//! ----------------------------------------------------------
//! Every thread probes the identical slot sequence for a given key, and by
//! (3) a slot's contents never change once set. So all inserters of key K make
//! the same skip/stop decision at every slot and converge on the same first
//! null slot. Exactly one `compare_exchange` there succeeds; every loser
//! re-reads that same slot, finds K, and degrades to an update.
//!
//! Tradeoff: (3) means the map cannot grow. This is what the previous code
//! already did in practice via `SHARD_PREALLOCATE`, but made explicit and
//! bounds-checked instead of a latent overrun. Size it with
//! [`CustomMap::with_capacity`]; [`CustomMap::try_insert`] reports saturation.

use std::cell::{Cell, UnsafeCell};
use std::hash::BuildHasher;
use std::marker::PhantomData;
use std::ops::Deref;
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering, fence};

use crossbeam_utils::CachePadded;
use foldhash::fast::RandomState;

// ─────────────────────────────────────────────────────────────────────────────
// Epoch-based reclamation
//
// Only the *value* path needs this. Entries and the slot array are never freed
// while the map lives, so `contains_key` never pins.
//
// A thread pins by publishing the current global epoch into its participant
// slot. The epoch may only advance once every active participant has been
// observed at the current epoch. Garbage retired at epoch `e` is therefore
// unreachable once the global epoch reaches `e + 2`: a thread pinned at `e`
// pins advancement at `e + 1`, so reaching `e + 2` proves every reader that
// could have loaded the pointer has since unpinned.
// ─────────────────────────────────────────────────────────────────────────────

mod ebr {
    use super::*;

    /// Reserved to mean "not pinned", so live epochs start at 1.
    const INACTIVE: u64 = 0;

    /// Retires between reclamation attempts; amortises the participant scan.
    ///
    /// A retired value is still protected by the two-epoch rule; this only
    /// controls how often the (comparatively expensive) participant scan is
    /// attempted.  Updates are much more common than reads in the write
    /// benchmark, so scanning every 64 replacements made the writer pay a
    /// measurable tax for no safety benefit.
    const COLLECT_INTERVAL: usize = 256;
    const INITIAL_GARBAGE_CAPACITY: usize = COLLECT_INTERVAL * 2;
    const INITIAL_VALUE_POOL_CAPACITY: usize = COLLECT_INTERVAL;

    static GLOBAL_EPOCH: AtomicU64 = AtomicU64::new(1);
    static PARTICIPANTS: AtomicPtr<Participant> = AtomicPtr::new(ptr::null_mut());
    static ORPHANS: AtomicPtr<OrphanNode> = AtomicPtr::new(ptr::null_mut());

    /// One per thread, pushed onto an intrusive lock-free list and never
    /// removed. A dead thread leaves its node behind marked INACTIVE, which
    /// never blocks epoch advancement.
    pub struct Participant {
        local: CachePadded<AtomicU64>,
        next: *mut Participant,
    }

    // SAFETY: `next` is written once before the node is published by CAS and is
    // read-only thereafter; `local` is atomic.
    unsafe impl Sync for Participant {}
    unsafe impl Send for Participant {}

    struct Garbage {
        ptr: *mut ValueBox,
        epoch: u64,
    }

    // SAFETY: each item is freed exactly once, by whichever thread collects it,
    // and only once provably unreachable from every thread.
    unsafe impl Send for Garbage {}

    struct OrphanNode {
        garbage: Vec<Garbage>,
        next: *mut OrphanNode,
    }

    /// Hand garbage from an exiting thread to whoever collects next.
    fn push_orphans(garbage: Vec<Garbage>) {
        if garbage.is_empty() {
            return;
        }
        let node = Box::into_raw(Box::new(OrphanNode { garbage, next: ptr::null_mut() }));
        loop {
            let head = ORPHANS.load(Ordering::Acquire);
            // SAFETY: not yet published, so we own it exclusively.
            unsafe { (*node).next = head };
            if ORPHANS
                .compare_exchange_weak(head, node, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return;
            }
        }
    }

    /// Returns true when orphan batches were appended. The relaxed fast check
    /// avoids a global atomic RMW on the overwhelmingly common empty path.
    fn adopt_orphans(dst: &mut Vec<Garbage>) -> bool {
        if ORPHANS.load(Ordering::Relaxed).is_null() {
            return false;
        }
        let mut p = ORPHANS.swap(ptr::null_mut(), Ordering::AcqRel);
        if p.is_null() {
            return false;
        }
        while !p.is_null() {
            // SAFETY: the swap took exclusive ownership of the whole list.
            let node = unsafe { Box::from_raw(p) };
            p = node.next;
            dst.extend(node.garbage);
        }
        true
    }

    struct Local {
        participant: &'static Participant,
        garbage: UnsafeCell<Vec<Garbage>>,
        /// Reentrancy count, so a nested pin does not unpin early.
        depth: Cell<usize>,
        retires: Cell<usize>,
        /// Reusable value headers that have passed the two-epoch grace period.
        /// Kept beside the retire list so replacement needs one TLS lookup,
        /// rather than separate EBR and allocator-cache lookups.
        value_pool: UnsafeCell<Vec<*mut ValueBox>>,
    }

    impl Local {
        fn new() -> Self {
            let participant = Box::into_raw(Box::new(Participant {
                local: CachePadded::new(AtomicU64::new(INACTIVE)),
                next: ptr::null_mut(),
            }));
            loop {
                let head = PARTICIPANTS.load(Ordering::Acquire);
                // SAFETY: not yet published.
                unsafe { (*participant).next = head };
                if PARTICIPANTS
                    .compare_exchange_weak(head, participant, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    break;
                }
            }
            Local {
                // SAFETY: leaked deliberately — participants outlive all threads.
                participant: unsafe { &*participant },
                garbage: UnsafeCell::new(Vec::with_capacity(INITIAL_GARBAGE_CAPACITY)),
                depth: Cell::new(0),
                retires: Cell::new(0),
                value_pool: UnsafeCell::new(Vec::with_capacity(INITIAL_VALUE_POOL_CAPACITY)),
            }
        }

        #[inline(always)]
        fn pin(&self) {
            let d = self.depth.get();
            self.depth.set(d + 1);
            if d == 0 {
                // Publish before loading a protected pointer.  A release
                // store followed by an acquire fence is enough for this
                // publication protocol and is substantially cheaper than a
                // SeqCst fence on weakly ordered CPUs.
                loop {
                    let e = GLOBAL_EPOCH.load(Ordering::Relaxed);
                    self.participant.local.store(e, Ordering::Release);
                    fence(Ordering::Acquire);

                    // If a collector advanced the epoch in the small window
                    // before it observed our publication, retry with the new
                    // epoch.  We do not load a value pointer until this check
                    // succeeds, so that race cannot expose reclaimed memory.
                    if GLOBAL_EPOCH.load(Ordering::Acquire) == e {
                        break;
                    }
                    self.participant.local.store(INACTIVE, Ordering::Release);
                }
            }
        }

        #[inline(always)]
        fn unpin(&self) {
            let d = self.depth.get() - 1;
            self.depth.set(d);
            if d == 0 {
                // Release: our reads cannot be reordered after going idle.
                self.participant.local.store(INACTIVE, Ordering::Release);
            }
        }

        fn collect(&self) {
            // SAFETY: thread-local, and never reentered — no reclaim callback
            // touches the EBR.
            let garbage = unsafe { &mut *self.garbage.get() };
            let adopted_orphans = adopt_orphans(garbage);

            let global = GLOBAL_EPOCH.load(Ordering::Acquire);

            // Advance only if every *active* participant is caught up.
            let mut all_caught_up = true;
            let mut p = PARTICIPANTS.load(Ordering::Acquire);
            while !p.is_null() {
                // SAFETY: participants are leaked and never freed.
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
            // Local retirement epochs are monotonic. Orphan adoption is rare
            // and may append older batches, so restore ordering only then.
            if adopted_orphans && !garbage.is_sorted_by_key(|item| item.epoch) {
                garbage.sort_unstable_by_key(|item| item.epoch);
            }
            let reclaimable = garbage.partition_point(|item| item.epoch + 2 <= safe);
            if reclaimable != 0 {
                let pool = unsafe { &mut *self.value_pool.get() };
                for item in garbage.drain(..reclaimable) {
                    if pool.len() < VALUE_POOL_LIMIT {
                        pool.push(item.ptr);
                    } else {
                        unsafe { free_value(item.ptr) };
                    }
                }
            }
        }

        #[inline(always)]
        fn replacement(&self, value: String) -> *mut ValueBox {
            // SAFETY: this vector is thread-local.
            let pool = unsafe { &mut *self.value_pool.get() };
            if let Some(p) = pool.pop() {
                // SAFETY: a pooled box has completed its EBR grace period and
                // is exclusively owned by this thread.
                unsafe {
                    ptr::drop_in_place(&mut (*p).0);
                    ptr::write(&mut (*p).0, value);
                }
                p
            } else {
                new_value(value)
            }
        }

        /// Build a replacement inside a reclaimed String buffer. This avoids
        /// allocating a new value buffer when a sufficiently large retired
        /// value has completed its grace period.
        #[inline(always)]
        fn replacement_with(&self, build: impl FnOnce(&mut String)) -> *mut ValueBox {
            // SAFETY: this vector and every pooled value are thread-local.
            let pool = unsafe { &mut *self.value_pool.get() };
            let p = if let Some(p) = pool.pop() {
                p
            } else {
                new_value(String::new())
            };
            // SAFETY: a pooled/new box is exclusively owned by this thread.
            let value = unsafe { &mut (*p).0 };
            value.clear();
            build(value);
            p
        }

        #[inline(always)]
        unsafe fn push_retired(&self, ptr: *mut ValueBox, epoch: u64) -> bool {
            let n = self.retires.get();
            // SAFETY: thread-local, no reentrancy.
            unsafe { &mut *self.garbage.get() }.push(Garbage { ptr, epoch });

            let next = n + 1;
            self.retires.set(next);
            next % COLLECT_INTERVAL == 0
        }

        #[inline(always)]
        unsafe fn retire_value_at(&self, ptr: *mut ValueBox, epoch: u64) {
            if unsafe { self.push_retired(ptr, epoch) } {
                self.collect();
            }
        }

        #[inline(always)]
        unsafe fn retire_value(&self, ptr: *mut ValueBox) {
            // This must be observed after unlinking unless the caller itself
            // is pinned at the supplied epoch (the batched writer path).
            let epoch = GLOBAL_EPOCH.load(Ordering::Acquire);
            unsafe { self.retire_value_at(ptr, epoch) };
        }
    }

    impl Drop for Local {
        fn drop(&mut self) {
            let garbage = self.garbage.get_mut();
            push_orphans(std::mem::take(garbage));
            for p in self.value_pool.get_mut().drain(..) {
                // SAFETY: the local pool owns each pointer exclusively.
                unsafe { free_value(p) };
            }
            self.participant.local.store(INACTIVE, Ordering::Release);
        }
    }

    thread_local! {
        static LOCAL: Local = Local::new();
    }

    /// Keeps every value box loaded during its lifetime alive.
    pub struct Guard {
        _not_send: std::marker::PhantomData<*const ()>,
    }

    /// Thread-local replacement context pinned at one epoch. Pinning prevents
    /// this retirement stamp from becoming prematurely reclaimable while a
    /// batch is in progress.
    pub struct Writer {
        local: &'static Local,
        epoch: u64,
        collect_on_drop: Cell<bool>,
        _not_send: std::marker::PhantomData<*const ()>,
    }

    impl Writer {
        #[inline(always)]
        pub fn replace(&self, slot: &AtomicPtr<ValueBox>, value: String) -> bool {
            let new = self.local.replacement(value);
            let old = slot.swap(new, Ordering::Release);
            if !old.is_null() && unsafe { self.local.push_retired(old, self.epoch) } {
                self.collect_on_drop.set(true);
            }
            old.is_null()
        }


        #[inline(always)]
        pub fn replace_with(
            &self,
            slot: &AtomicPtr<ValueBox>,
            build: impl FnOnce(&mut String),
        ) -> bool {
            let new = self.local.replacement_with(build);
            let old = slot.swap(new, Ordering::Release);
            if !old.is_null() && unsafe { self.local.push_retired(old, self.epoch) } {
                self.collect_on_drop.set(true);
            }
            old.is_null()
        }
    }

    impl Drop for Writer {
        #[inline]
        fn drop(&mut self) {
            // Going inactive first lets this collection complete epoch
            // progress instead of observing its own stale pin.
            self.local.unpin();
            if self.collect_on_drop.get() {
                self.local.collect();
            }
        }
    }

    impl Drop for Guard {
        #[inline(always)]
        fn drop(&mut self) {
            LOCAL.with(|l| l.unpin());
        }
    }

    #[inline(always)]
    pub fn pin() -> Guard {
        LOCAL.with(|l| l.pin());
        Guard { _not_send: std::marker::PhantomData }
    }

    #[inline]
    pub fn writer() -> Writer {
        LOCAL.with(|local| {
            local.pin();
            let epoch = local.participant.local.load(Ordering::Relaxed);
            Writer {
                // SAFETY: `Local` is TLS storage and lives until thread exit;
                // `Writer` is !Send through its contained `Guard`.
                local: unsafe { &*(local as *const Local) },
                epoch,
                collect_on_drop: Cell::new(false),
                _not_send: std::marker::PhantomData,
            }
        })
    }

    /// Replace one immutable value and defer the old box for EBR reuse.  The
    /// allocation cache, pointer swap, and retirement bookkeeping share one
    /// TLS lookup on the common update path.
    #[inline(always)]
    pub fn replace_value(slot: &AtomicPtr<ValueBox>, value: String) -> bool {
        LOCAL.with(|l| {
            let new = l.replacement(value);
            let old = slot.swap(new, Ordering::Release);
            if old.is_null() {
                return true;
            }
            // SAFETY: `old` was unlinked by the swap and is retired exactly
            // once by this writer.
            unsafe { l.retire_value(old) };
            false
        })
    }

    /// Retire an already-unlinked value (used only after a lost insertion CAS).
    ///
    /// # Safety
    /// `ptr` must be unreachable from every new value-pointer load.
    #[inline(always)]
    pub unsafe fn retire_value(ptr: *mut ValueBox) {
        if ptr.is_null() {
            return;
        }
        LOCAL.with(|l| unsafe { l.retire_value(ptr) });
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Value box — the only mutable, reclaimed allocation
// ─────────────────────────────────────────────────────────────────────────────

struct ValueBox(String);

const VALUE_POOL_LIMIT: usize = 4096;

#[inline]
fn new_value(v: String) -> *mut ValueBox {
    Box::into_raw(Box::new(ValueBox(v)))
}

/// # Safety
/// `p` must come from [`new_value`] and must not have been freed.
unsafe fn free_value(p: *mut ValueBox) {
    drop(unsafe { Box::from_raw(p) });
}

// ─────────────────────────────────────────────────────────────────────────────
// Entry — immutable except for `value`; never freed while the map lives
// ─────────────────────────────────────────────────────────────────────────────

/// Deliberately has no `Drop` impl. An unpublished entry that loses a CAS is
/// freed as a plain `Box<Entry>`, and that must *not* free the value box it
/// carries — by then the box may already be published in the winning entry.
/// Published entries are torn down explicitly in `Shard::drop`.
struct Entry {
    hash: u64,
    /// Swapped wholesale on update; the previous box goes to EBR.
    value: AtomicPtr<ValueBox>,
    key: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Shard — fixed-capacity open-addressed table, linear probing
// ─────────────────────────────────────────────────────────────────────────────

/// Max load factor as a fraction of capacity. Linear probing degrades sharply
/// past ~0.7, and a guaranteed-null slot is what terminates every probe loop.
const LOAD_NUM: usize = 7;
const LOAD_DEN: usize = 10;

/// Per-shard slot count when the caller does not size the map. Sized so the
/// existing benchmark (~16k keys per shard, since shard count scales with CPU
/// count) stays comfortably under the load ceiling.
const DEFAULT_SHARD_CAPACITY: usize = 32_768;

#[repr(align(128))]
struct Shard {
    slots: Box<[AtomicPtr<Entry>]>,
    mask: usize,
    /// Occupancy, including reservations not yet published. Held below
    /// `threshold` so a null slot always exists on every probe sequence.
    len: CachePadded<AtomicUsize>,
    threshold: usize,
}

impl Shard {
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

    /// Wait-free. Terminates because occupancy is capped below capacity, so a
    /// null slot always exists on the probe sequence.
    #[inline(always)]
    fn find(&self, key: &str, hash: u64) -> Option<&Entry> {
        self.find_index(key, hash).map(|(entry, _)| entry)
    }

    #[inline(always)]
    fn find_index(&self, key: &str, hash: u64) -> Option<(&Entry, usize)> {
        let mut i = (hash as usize) & self.mask;
        loop {
            // SAFETY: `i` is masked into range.
            let p = unsafe { self.slots.get_unchecked(i) }.load(Ordering::Acquire);
            if p.is_null() {
                return None;
            }
            // SAFETY: a published entry is never freed while the map lives.
            let e = unsafe { &*p };
            if e.hash == hash && &*e.key == key {
                return Some((e, i));
            }
            i = (i + 1) & self.mask;
        }
    }

    /// Fast path for a slot resolved by `prepare()`. Entries and slot
    /// positions never move, so successful prepared lookups need one slot load
    /// and one key validation. Missing/stale preparations fall back to probe.
    #[inline(always)]
    fn find_prepared(&self, key: &str, hash: u64, slot_index: usize) -> Option<&Entry> {
        if slot_index <= self.mask {
            let p = unsafe { self.slots.get_unchecked(slot_index) }.load(Ordering::Acquire);
            if !p.is_null() {
                let entry = unsafe { &*p };
                if entry.hash == hash && &*entry.key == key {
                    return Some(entry);
                }
            }
        }
        self.find(key, hash)
    }

    /// Lock-free insert with a precomputed hash (hash-once).
    ///
    /// `Ok(true)` if newly inserted, `Ok(false)` if an existing value was
    /// replaced, `Err(Full)` at the load ceiling.
    #[inline(always)]
    fn insert_hashed(&self, key: String, value: String, hash: u64) -> Result<bool, Full> {
        // SET overwhelmingly targets keys that already exist.  Do this probe
        // before allocating the candidate Entry (and its key) so an update
        // only allocates the replacement value.  A concurrent inserter is
        // harmless: the insertion loop below converges on its entry and
        // performs the same update there.
        if let Some(existing) = self.find(&key, hash) {
            return Ok(ebr::replace_value(&existing.value, value));
        }

        self.insert_new_hashed(key, value, hash)
    }

    /// Cold path: allocation, occupancy reservation, and publication for a
    /// key that was absent from the initial hot update probe.
    #[cold]
    #[inline(never)]
    fn insert_new_hashed(&self, key: String, value: String, hash: u64) -> Result<bool, Full> {

        // A stale value is fine here; the reservation below performs the
        // authoritative capacity check.  This fast rejection avoids building
        // and immediately destroying two heap allocations once a shard is
        // saturated.
        if self.len.load(Ordering::Relaxed) >= self.threshold {
            return Err(Full);
        }

        let vb = new_value(value);

        // Allocated once up front so a lost CAS never rebuilds the key, and so
        // `entry.key` can serve as the comparison key for the whole probe.
        let entry: *mut Entry = Box::into_raw(Box::new(Entry {
            hash,
            value: AtomicPtr::new(vb),
            key,
        }));
        // SAFETY: we solely own `entry` until it is published. Not used after
        // the `Box::from_raw` on the update path below.
        let key: &str = unsafe { &(*entry).key };
        let mut reserved = false;

        let mut i = (hash as usize) & self.mask;
        loop {
            // SAFETY: `i` is masked into range.
            let slot = unsafe { self.slots.get_unchecked(i) };

            // Keep the result.  The previous code loaded the same atomic a
            // second time immediately before dereferencing every occupied
            // slot, doubling the probe's atomic-load work on the write path.
            let p = slot.load(Ordering::Acquire);
            if p.is_null() {
                // Reserve before publishing, so occupancy can never exceed the
                // threshold that guarantees a null probe terminator.
                if !reserved {
                    let mut cur = self.len.load(Ordering::Relaxed);
                    loop {
                        if cur >= self.threshold {
                            // SAFETY: neither was ever published.
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
                // Lost this slot. Re-examine the *same* index: the winner may
                // hold our key, which turns this into an update.
                continue;
            }

            // SAFETY: non-null, and published entries are never freed.
            let e = unsafe { &*p };
            if e.hash == hash && e.key == key {
                // Update in place. Every racing writer of this key converges
                // here, so the last swap wins and no duplicate is created.
                let old = e.value.swap(vb, Ordering::AcqRel);

                if reserved {
                    self.len.fetch_sub(1, Ordering::Relaxed);
                }
                // Free our unpublished entry. `Entry` has no `Drop`, so this
                // frees only the key — `vb` is published above and untouched.
                // SAFETY: never published; `key` is not used after this point.
                unsafe { drop(Box::from_raw(entry)) };
                // SAFETY: `old` is unlinked, and is freed exactly once, after
                // every reader pinned before the swap has unpinned.
                let resurrected = old.is_null();
                if !resurrected {
                    unsafe { ebr::retire_value(old) };
                }
                return Ok(resurrected);
            }

            i = (i + 1) & self.mask;
        }
    }
}

impl Drop for Shard {
    fn drop(&mut self) {
        // Sole owner, so relaxed loads and direct frees are fine.
        for slot in self.slots.iter() {
            let p = slot.load(Ordering::Relaxed);
            if p.is_null() {
                continue;
            }
            // SAFETY: published entries are owned by the shard, freed once.
            let entry = unsafe { Box::from_raw(p) };
            let v = entry.value.load(Ordering::Relaxed);
            if !v.is_null() {
                // SAFETY: the live value box, owned by this entry.
                unsafe { free_value(v) };
            }
        }
    }
}

// SAFETY: all shared mutation goes through atomics; `Entry` and `ValueBox`
// contain only `Send + Sync` data.
unsafe impl Sync for Shard {}
unsafe impl Send for Shard {}

// ─────────────────────────────────────────────────────────────────────────────
// CustomMap
// ─────────────────────────────────────────────────────────────────────────────

/// The shard reached its load-factor ceiling. The map does not resize —
/// construct it with enough room via [`CustomMap::with_capacity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Full;

/// Returned by borrowed-key update APIs when the key does not exist. The
/// value is returned intact so the caller may insert it without rebuilding it.
#[derive(Debug)]
pub struct NotFound(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotFoundKey;

impl std::fmt::Display for Full {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CustomMap shard is full (fixed capacity, no resize)")
    }
}

/// A zero-copy value reference. It contains only a pointer and an epoch guard;
/// the underlying `String` is never cloned. Keeping this value alive keeps the
/// pointed-to allocation alive even when another thread replaces the key.
pub struct ValueRef<'a> {
    ptr: *const ValueBox,
    _guard: ebr::Guard,
    _map: PhantomData<&'a CustomMap>,
}

/// Pins reclamation once for a batch of zero-copy reads. This removes the TLS
/// lookup and publication fence from every individual lookup. Keep batches
/// reasonably short so writers can reclaim replaced values promptly.
pub struct ReadGuard<'a> {
    map: &'a CustomMap,
    _guard: ebr::Guard,
}

/// Hash and shard routing cached for a key. It is valid only for the map that
/// produced it because each map owns a randomized hash seed.
#[derive(Clone, Copy, Debug)]
pub struct PreparedKey {
    hash: u64,
    shard: usize,
    slot: usize,
}

/// Batches existing-key replacements through one TLS lookup and one epoch
/// publication. Drop after a short batch to allow reclamation to advance.
pub struct WriteGuard<'a> {
    map: &'a CustomMap,
    writer: ebr::Writer,
}

impl WriteGuard<'_> {
    #[inline(always)]
    pub fn update_prepared(
        &self,
        key: &str,
        prepared: PreparedKey,
        value: String,
    ) -> Result<(), NotFound> {
        let Some(shard) = self.map.shards.get(prepared.shard) else {
            return Err(NotFound(value));
        };
        let Some(existing) = shard.find_prepared(key, prepared.hash, prepared.slot) else {
            return Err(NotFound(value));
        };
        if self.writer.replace(&existing.value, value) {
            self.map.key_count.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }


    /// Replace a value by filling a reclaimed String buffer. The callback must
    /// fully construct the new value; its buffer is empty on entry.
    #[inline(always)]
    pub fn update_prepared_with(
        &self,
        key: &str,
        prepared: PreparedKey,
        build: impl FnOnce(&mut String),
    ) -> Result<(), NotFoundKey> {
        let Some(shard) = self.map.shards.get(prepared.shard) else {
            return Err(NotFoundKey);
        };
        let Some(existing) = shard.find_prepared(key, prepared.hash, prepared.slot) else {
            return Err(NotFoundKey);
        };
        if self.writer.replace_with(&existing.value, build) {
            self.map.key_count.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }
}

impl ReadGuard<'_> {
    /// Returns a value borrowed for at most the lifetime of this guard.
    #[inline(always)]
    pub fn get<'g>(&'g self, key: &str) -> Option<&'g str> {
        self.get_prepared(key, self.map.prepare(key))
    }

    /// Zero-copy lookup that skips hashing and shard calculation. Use this for
    /// hot/repeated keys prepared by the same map.
    #[inline(always)]
    pub fn get_prepared<'g>(&'g self, key: &str, prepared: PreparedKey) -> Option<&'g str> {
        let e = self
            .map
            .shards
            .get(prepared.shard)?
            .find_prepared(key, prepared.hash, prepared.slot)?;
        let ptr = e.value.load(Ordering::Acquire);
        if ptr.is_null() {
            return None;
        }
        // SAFETY: this guard remains pinned for `'g`, and `map` cannot be
        // dropped before the guard.
        Some(unsafe { (*ptr).0.as_str() })
    }

    #[inline(always)]
    pub fn contains_key(&self, key: &str) -> bool {
        self.contains_prepared(key, self.map.prepare(key))
    }

    #[inline(always)]
    pub fn contains_prepared(&self, key: &str, prepared: PreparedKey) -> bool {
        self.map
            .shards
            .get(prepared.shard)
            .map(|shard| shard.find_prepared(key, prepared.hash, prepared.slot))
            .flatten()
            .is_some_and(|entry| !entry.value.load(Ordering::Acquire).is_null())
    }
}

impl Deref for ValueRef<'_> {
    type Target = str;

    #[inline(always)]
    fn deref(&self) -> &str {
        // SAFETY: `_guard` pins reclamation and `_map` prevents the map from
        // being dropped while this reference exists.
        unsafe { (*self.ptr).0.as_str() }
    }
}

impl AsRef<str> for ValueRef<'_> {
    #[inline(always)]
    fn as_ref(&self) -> &str {
        self
    }
}

impl std::fmt::Debug for ValueRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, f)
    }
}

impl std::error::Error for Full {}

pub struct CustomMap {
    shards: Box<[Shard]>,
    /// Shard index comes from the *high* bits so it stays independent of the
    /// low bits the per-shard slot index consumes.
    shift: u32,
    shard_mask: usize,
    hasher: RandomState,
    key_count: CachePadded<AtomicUsize>,
}

impl CustomMap {
    /// Per-shard capacity defaults to 32768 slots. Prefer
    /// [`CustomMap::with_capacity`] to size the load factor to your data.
    pub fn with_shards(shard_count: usize) -> Self {
        Self::build(shard_count.next_power_of_two().max(1), DEFAULT_SHARD_CAPACITY)
    }

    /// `expected_keys` is the total across all shards. Per-shard capacity is
    /// derived with headroom for the load factor and for uneven hash spread.
    pub fn with_capacity(shard_count: usize, expected_keys: usize) -> Self {
        let n = shard_count.next_power_of_two().max(1);
        let keys_per_shard = expected_keys.div_ceil(n);
        let at_load_limit = keys_per_shard
            .saturating_mul(LOAD_DEN)
            .div_ceil(LOAD_NUM);
        // 25% protects against ordinary shard skew. The old unconditional 2×
        // multiplier frequently forced the next power-of-two twice as high.
        let per_shard = at_load_limit.saturating_mul(5).div_ceil(4);
        Self::build(n, per_shard.max(8))
    }

    fn build(shards: usize, per_shard: usize) -> Self {
        // Clamped to 63: `hash >> 64` would be a shift overflow when there is
        // only one shard. Masking afterwards makes the clamp harmless.
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

    /// Precompute routing for a hot key. The returned token is only valid with
    /// this map instance.
    #[inline(always)]
    pub fn prepare(&self, key: &str) -> PreparedKey {
        let (hash, shard) = self.locate(key);
        let slot = self.shards[shard]
            .find_index(key, hash)
            .map_or(usize::MAX, |(_, slot)| slot);
        PreparedKey { hash, shard, slot }
    }

    /// Wait-free: no lock, no atomic RMW, no fence, no retry.
    #[inline]
    pub fn contains_key(&self, key: &str) -> bool {
        let (h, idx) = self.locate(key);
        // SAFETY: `locate` masks the index into range.
        unsafe { self.shards.get_unchecked(idx) }
            .find(key, h)
            .is_some_and(|entry| !entry.value.load(Ordering::Acquire).is_null())
    }

    /// Lock-free; clones the value under an epoch guard.
    #[inline]
    pub fn get(&self, key: &str) -> Option<String> {
        self.get_ref(key).map(|v| v.to_owned())
    }

    /// Returns a borrowed, zero-copy value protected by an epoch guard.
    /// This is the preferred read API when the caller can finish using the
    /// value before moving it to another thread or storing it long-term.
    #[inline]
    pub fn get_ref(&self, key: &str) -> Option<ValueRef<'_>> {
        let (h, idx) = self.locate(key);
        let guard = ebr::pin();
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

    /// Pin once for a batch of zero-copy reads. A batch size around 64–1024
    /// lookups normally amortizes pinning without retaining much garbage.
    #[inline]
    pub fn read(&self) -> ReadGuard<'_> {
        ReadGuard {
            map: self,
            _guard: ebr::pin(),
        }
    }

    /// Pin once for a short batch of existing-key updates.
    #[inline]
    pub fn write(&self) -> WriteGuard<'_> {
        WriteGuard {
            map: self,
            writer: ebr::writer(),
        }
    }

    /// Lock-free and allocation-free: hands the value to `f` in place, skipping
    /// the malloc + memcpy that [`CustomMap::get`] pays.
    #[inline]
    pub fn with_value<R>(&self, key: &str, f: impl FnOnce(&str) -> R) -> Option<R> {
        self.get_ref(key).map(|v| f(&v))
    }

    /// Lock-free. Returns `true` if the key was newly inserted.
    ///
    /// # Panics
    /// If the target shard is at its load-factor ceiling. Use
    /// [`CustomMap::try_insert`] to handle that instead.
    #[inline]
    pub fn insert(&self, key: String, value: String) -> bool {
        match self.try_insert(key, value) {
            Ok(is_new) => is_new,
            Err(Full) => panic!(
                "CustomMap shard full — construct with \
                 CustomMap::with_capacity(shards, expected_keys)"
            ),
        }
    }

    /// Lock-free. `Err(Full)` when the shard is at its load-factor ceiling.
    #[inline]
    pub fn try_insert(&self, key: String, value: String) -> Result<bool, Full> {
        let (h, idx) = self.locate(&key);
        // SAFETY: `locate` masks the index into range.
        let is_new = unsafe { self.shards.get_unchecked(idx) }.insert_hashed(key, value, h)?;
        if is_new {
            self.key_count.fetch_add(1, Ordering::Relaxed);
        }
        Ok(is_new)
    }

    /// Replace an existing value without allocating or cloning the key.
    /// Returns the provided value when the key is absent.
    #[inline]
    pub fn update(&self, key: &str, value: String) -> Result<(), NotFound> {
        self.update_prepared(key, self.prepare(key), value)
    }

    /// Pre-hashed form of [`CustomMap::update`]. This is the lowest-overhead
    /// write path for frequently updated keys.
    #[inline]
    pub fn update_prepared(
        &self,
        key: &str,
        prepared: PreparedKey,
        value: String,
    ) -> Result<(), NotFound> {
        let Some(shard) = self.shards.get(prepared.shard) else {
            return Err(NotFound(value));
        };
        let Some(existing) = shard.find_prepared(key, prepared.hash, prepared.slot) else {
            return Err(NotFound(value));
        };
        if ebr::replace_value(&existing.value, value) {
            self.key_count.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Logically removes a key with one atomic pointer swap. The immutable key
    /// entry remains as a tombstone so unpinned lookups never dereference freed
    /// entry memory. Reinserting the same key reuses this slot.
    #[inline]
    pub fn remove(&self, key: &str) -> Option<String> {
        let prepared = self.prepare(key);
        let entry = self
            .shards
            .get(prepared.shard)?
            .find_prepared(key, prepared.hash, prepared.slot)?;
        let _guard = ebr::pin();
        let old = entry.value.swap(ptr::null_mut(), Ordering::AcqRel);
        if old.is_null() {
            return None;
        }
        // Clone for DashMap-like ownership semantics. The epoch guard keeps the
        // old allocation valid while copying, then retirement defers freeing.
        let value = unsafe { (*old).0.clone() };
        self.key_count.fetch_sub(1, Ordering::Relaxed);
        unsafe { ebr::retire_value(old) };
        Some(value)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.key_count.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// SAFETY: every shared access is atomic; contents are `Send + Sync`.
unsafe impl Sync for CustomMap {}
unsafe impl Send for CustomMap {}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::thread;

    #[test]
    fn insert_get_update() {
        let m = CustomMap::with_capacity(4, 1024);
        assert!(m.insert("a".into(), "1".into()));
        assert!(!m.insert("a".into(), "2".into()), "second insert is an update");
        assert_eq!(m.get("a").as_deref(), Some("2"));
        assert_eq!(m.len(), 1);
        assert!(m.contains_key("a"));
        assert!(!m.contains_key("b"));
        assert_eq!(m.get("b"), None);
    }

    #[test]
    fn remove_hides_key_and_same_key_reinsertion_reuses_entry() {
        let m = CustomMap::with_capacity(2, 32);
        m.insert("key".into(), "old".into());
        assert_eq!(m.remove("key").as_deref(), Some("old"));
        assert_eq!(m.len(), 0);
        assert!(!m.contains_key("key"));
        assert_eq!(m.get_ref("key").as_deref(), None);
        assert_eq!(m.remove("key"), None);

        assert!(m.insert("key".into(), "new".into()));
        assert_eq!(m.len(), 1);
        assert_eq!(m.get_ref("key").as_deref(), Some("new"));
    }

    #[test]
    fn concurrent_remove_has_single_winner() {
        const THREADS: usize = 16;
        let m = Arc::new(CustomMap::with_capacity(4, 64));
        m.insert("key".into(), "value".into());
        let winners = Arc::new(AtomicUsize::new(0));
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let m = Arc::clone(&m);
                let winners = Arc::clone(&winners);
                thread::spawn(move || {
                    if m.remove("key").is_some() {
                        winners.fetch_add(1, Ordering::Relaxed);
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(winners.load(Ordering::Relaxed), 1);
        assert_eq!(m.len(), 0);
    }

    #[test]
    fn borrowed_value_is_zero_copy_and_stays_valid_after_update() {
        let m = CustomMap::with_capacity(1, 16);
        m.insert("key".into(), "old-value".into());
        let borrowed = m.get_ref("key").expect("value present");
        assert_eq!(&*borrowed, "old-value");
        m.insert("key".into(), "new-value".into());
        assert_eq!(&*borrowed, "old-value");
        assert_eq!(m.get_ref("key").as_deref(), Some("new-value"));
    }

    #[test]
    fn batch_read_guard_serves_multiple_zero_copy_reads() {
        let m = CustomMap::with_capacity(2, 32);
        m.insert("a".into(), "one".into());
        m.insert("b".into(), "two".into());
        let guard = m.read();
        assert_eq!(guard.get("a"), Some("one"));
        assert_eq!(guard.get("b"), Some("two"));
        assert_eq!(guard.get("missing"), None);
        assert!(guard.contains_key("a"));
    }

    #[test]
    fn prepared_borrowed_key_read_and_update() {
        let m = CustomMap::with_capacity(2, 32);
        m.insert("hot-key".into(), "old".into());
        let prepared = m.prepare("hot-key");
        m.update_prepared("hot-key", prepared, "new".into())
            .unwrap();
        let guard = m.read();
        assert_eq!(guard.get_prepared("hot-key", prepared), Some("new"));

        let missing = m.prepare("missing");
        let err = m
            .update_prepared("missing", missing, "kept".into())
            .unwrap_err();
        assert_eq!(err.0, "kept");

        let writer = m.write();
        writer
            .update_prepared("hot-key", prepared, "batched".into())
            .unwrap();
        drop(writer);
        assert_eq!(m.get_ref("hot-key").as_deref(), Some("batched"));
    }

    #[test]
    fn reclaimed_buffer_updates_publish_complete_values() {
        use std::fmt::Write;

        let m = Arc::new(CustomMap::with_capacity(4, 64));
        m.insert("key".into(), "initial".into());
        let prepared = m.prepare("key");
        for batch in 0..100usize {
            let writer = m.write();
            for i in 0..1024usize {
                writer
                    .update_prepared_with("key", prepared, |value| {
                        write!(value, "batch:{batch}:value:{i}:{}", "x".repeat(i % 64)).unwrap();
                    })
                    .unwrap();
            }
        }
        let value = m.get_ref("key").unwrap();
        assert!(value.starts_with("batch:99:value:1023:"));
        assert!(value.ends_with(&"x".repeat(63)));
    }

    #[test]
    fn single_shard_does_not_overflow_shift() {
        let m = CustomMap::with_capacity(1, 64);
        for i in 0..32 {
            m.insert(format!("k{i}"), format!("v{i}"));
        }
        for i in 0..32 {
            assert_eq!(m.get(&format!("k{i}")).as_deref(), Some(&*format!("v{i}")));
        }
    }

    #[test]
    fn reports_full_instead_of_overrunning() {
        let m = CustomMap::with_capacity(1, 8);
        let mut inserted = 0usize;
        for i in 0..10_000 {
            match m.try_insert(format!("k{i}"), "v".into()) {
                Ok(_) => inserted += 1,
                Err(Full) => break,
            }
        }
        assert!(inserted > 0);
        assert!(inserted < 10_000, "must saturate rather than overrun");
        // Everything accepted must still be readable.
        for i in 0..inserted {
            assert!(m.contains_key(&format!("k{i}")));
        }
    }

    #[test]
    fn randomized_operations_match_std_hashmap() {
        use std::collections::HashMap;

        let m = CustomMap::with_capacity(8, 4096);
        let mut reference = HashMap::<String, String>::new();
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        for step in 0..100_000usize {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let key = format!("k{}", state as usize % 1024);
            if state & 3 != 0 {
                let value = format!("v:{step}:{state}");
                let expected_new = reference.insert(key.clone(), value.clone()).is_none();
                assert_eq!(m.insert(key, value), expected_new);
            } else {
                assert_eq!(m.get_ref(&key).as_deref(), reference.get(&key).map(String::as_str));
            }
            assert_eq!(m.len(), reference.len());
        }
    }

    #[test]
    fn concurrent_disjoint_inserts_all_land() {
        const T: usize = 8;
        const N: usize = 20_000;
        let m = Arc::new(CustomMap::with_capacity(64, T * N));
        let hs: Vec<_> = (0..T)
            .map(|t| {
                let m = Arc::clone(&m);
                thread::spawn(move || {
                    for i in 0..N {
                        let k = t * N + i;
                        assert!(m.insert(format!("key:{k}"), format!("val:{k}")));
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        assert_eq!(m.len(), T * N);
        for k in 0..T * N {
            assert_eq!(m.get(&format!("key:{k}")).as_deref(), Some(&*format!("val:{k}")));
        }
    }

    /// The invariant that matters: N threads racing on the *same* keys must
    /// produce exactly one entry each, never a duplicate.
    #[test]
    fn concurrent_same_key_inserts_never_duplicate() {
        const T: usize = 8;
        const KEYS: usize = 2_000;
        const ROUNDS: usize = 5;
        let m = Arc::new(CustomMap::with_capacity(16, KEYS * 4));
        let new_count = Arc::new(AtomicUsize::new(0));

        let hs: Vec<_> = (0..T)
            .map(|t| {
                let m = Arc::clone(&m);
                let new_count = Arc::clone(&new_count);
                thread::spawn(move || {
                    for _ in 0..ROUNDS {
                        for k in 0..KEYS {
                            if m.insert(format!("key:{k}"), format!("from:{t}")) {
                                new_count.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }

        assert_eq!(new_count.load(Ordering::Relaxed), KEYS, "one insert per key");
        assert_eq!(m.len(), KEYS);
        for k in 0..KEYS {
            let v = m.get(&format!("key:{k}")).expect("present");
            assert!(v.starts_with("from:"), "value must not be torn: {v:?}");
        }
    }

    /// Readers cloning values while writers replace them: the case that was a
    /// use-after-free under the old seqlock. Run under Miri/ASan to be sure.
    #[test]
    fn readers_survive_concurrent_value_replacement() {
        const KEYS: usize = 256;
        let m = Arc::new(CustomMap::with_capacity(8, KEYS * 4));
        for k in 0..KEYS {
            m.insert(format!("key:{k}"), "initial".into());
        }
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let writers: Vec<_> = (0..4)
            .map(|t| {
                let m = Arc::clone(&m);
                let stop = Arc::clone(&stop);
                thread::spawn(move || {
                    let mut n = 0usize;
                    while !stop.load(Ordering::Relaxed) {
                        let k = n % KEYS;
                        // Vary length so a torn read would be visible.
                        m.insert(format!("key:{k}"), "x".repeat(1 + (n % 64)));
                        n += 1;
                    }
                })
            })
            .collect();

        let readers: Vec<_> = (0..4)
            .map(|_| {
                let m = Arc::clone(&m);
                thread::spawn(move || {
                    for n in 0..200_000usize {
                        let k = n % KEYS;
                        if let Some(v) = m.get(&format!("key:{k}")) {
                            assert!(
                                v == "initial" || v.bytes().all(|b| b == b'x'),
                                "torn value: {v:?}"
                            );
                        }
                    }
                })
            })
            .collect();

        for h in readers {
            h.join().unwrap();
        }
        stop.store(true, Ordering::Relaxed);
        for h in writers {
            h.join().unwrap();
        }
        assert_eq!(m.len(), KEYS);
    }

    #[test]
    fn long_lived_borrow_blocks_reuse_not_writers() {
        let m = Arc::new(CustomMap::with_capacity(4, 64));
        m.insert("key".into(), "held".into());
        let held = m.get_ref("key").unwrap();

        let writer_map = Arc::clone(&m);
        let writer = thread::spawn(move || {
            for n in 0..50_000usize {
                writer_map.insert("key".into(), format!("value-{n}"));
            }
        });
        writer.join().unwrap();

        assert_eq!(&*held, "held");
        assert_eq!(m.len(), 1);
        assert!(m.get_ref("key").unwrap().starts_with("value-"));
    }
}
