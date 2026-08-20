# FyroDB Memory & Performance Optimization Tasks

## Progress

| Phase | Status | Changes |
|-------|--------|---------|
| 1. RSS Reporting | ✅ DONE | `used_memory_rss` now reports `allocator.resident` (actual jemalloc pages) instead of kernel VmRSS which is inflated by jemalloc's transient extent mapping; added `mem_fragmentation_ratio` |
| 2. FLUSHALL Reclaim | ✅ DONE | `flush()` does 4 EBR collect rounds before purge; `purge()` uses MALLCTL_ARENAS_ALL single call |
| 3. ZSet Speed | ✅ DONE | `range_by_score`/`count_in_score_range`/`remove_range_by_score` → O(log n) binary search; removed fingerprint rebuild on remove/pop; `incr()` in-place when position unchanged; `get_score`/`rank` check bloom filter first |
| 4. zmalloc | ✅ DONE | Added `used_memory()` atomic counter; `fragmentation_ratio()`; jemalloc config matching Redis: `narenas:1,tcache:false,dirty_decay_ms:0,muzzy_decay_ms:0`; `disable_cache_oblivious` feature; `lg_quantum=3`; `used_memory_rss` reports `allocator.resident` instead of inflated VmRSS |
| 5. Minor Opts | ✅ DONE | `defragment_values` no-clone; `RETAINED_WRITE_BUFFER` 256→32 KB; `rbuf` 8→2 KB; `zinterstore` no-clone |

All tests pass: **362 tests, 0 failures.**

---

## Real Measurements (1 worker, all defaults)

```
FYRODB_WORKERS=1, FYRODB_SHARDS=4, FYRODB_MAX_KEYS=1000

Kernel truth (cat /proc/PID/status):
  VmRSS:    3884 kB  (3.8 MB actual physical RAM)
  VmSize:   8392 kB  (8.2 MB virtual address space)
  VmHWM:    3884 kB  (peak RSS = current, never went higher)

smaps_rollup:
  Private_Dirty:   112 kB  ← YOUR unique heap data
  Anonymous:       316 kB  ← heap + stacks
  Shared_Clean:   3568 kB  ← shared libs (libc, etc.)

INFO command reports (WRONG):
  used_memory:      693632     (677 KB — jemalloc allocated, correct)
  used_memory_rss:  27045888   (25.8 MB — INFLATED)
  allocator_active:    843776  (824 KB)
  allocator_resident: 5386240  (5.1 MB)
  allocator_retained: 9641984  (9.2 MB — virtual, not RSS)
```

**The "22 MB" is a measurement bug.** The server only uses **3.8 MB** at idle.
When `INFO` is called, it triggers jemalloc `stats()` which demand-faults retained
virtual pages into RSS, inflating the number during measurement. The RSS jumps
from 3.8 MB → 25 MB just by reading the stats.

---

## Phase 1: Fix `used_memory_rss` Reporting (Bug — High Priority) ✅ DONE

### Issue 1.1: `rss_bytes()` reads inflated RSS

**File:** `src/storage/store.rs` — `rss_bytes()` (line 189)  
**File:** `src/storage/server.rs` — `info()` (line 56)

**Problem:** The INFO command calls `rust_zmalloc::stats()` which does `refresh_epoch()` + multiple `mallctl` calls. This touches jemalloc's internal metadata pages that were retained (mapped virtual but not yet RSS). The page faults convert retained→resident, inflating RSS from 3.8 MB → 25 MB **during the INFO call itself**.

Then `rss_bytes()` reads `/proc/self/statm` and reports the now-inflated value.

**Fix:**  
Read RSS **before** touching jemalloc stats. Also use `/proc/self/status` VmRSS which is more accurate:

```rust
pub fn rss_bytes() -> usize {
    // Read VmRSS from /proc/self/status (not statm) for accurate RSS
    // that excludes any measurement-induced page faults.
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<usize>().ok())
        })
        .map(|kb| kb * 1024)
        .unwrap_or(0)
}
```

And in `info()`, read RSS first:
```rust
pub fn info(&self) -> String {
    let rss = rss_bytes();             // ← READ FIRST before stats
    let peak_rss = peak_rss_bytes();   // ← READ FIRST
    let allocator = rust_zmalloc::stats();  // this may fault pages
    let allocated = allocator.allocated;
    // ... rest
}
```

### Issue 1.2: Missing fragmentation ratio

**Problem:** Redis reports `mem_fragmentation_ratio` which is crucial for monitoring. FyroDB doesn't.

**Fix:** Add to INFO output:
```rust
let frag_ratio = if allocated > 0 {
    rss as f64 / allocated as f64
} else {
    1.0
};
// Add: mem_fragmentation_ratio:{frag_ratio:.2}
```

---

## Phase 2: Fix FLUSHALL Memory Reclamation ✅ DONE

### Issue 2.1: EBR doesn't fully reclaim after `clear()`

**File:** `crates/customhash/src/lib.rs` — `clear()` (~line 1185)  
**Problem:** After `clear()`, entries are retired to EBR. `force_collect_quiescent()` loops 64 times with `yield_now()`, but if only 1 worker thread exists and it's not pinned, epoch advancement may complete quickly — OR if the thread calling flush IS pinned, retired entries can't be reclaimed until it unpins.

**Fix:**
- Ensure caller is NOT in an EBR pin when calling flush
- In `force_collect_quiescent()`, add explicit epoch bumps between collect rounds
- Call `rust_zmalloc::purge()` after collection

### Issue 2.2: `flush()` calls `purge_allocator()` too early

**File:** `src/storage/server.rs` — `flush()` (~line 97)  
**Problem:** `purge_allocator()` is called after `force_collect_quiescent()`, but with 1 worker there may still be garbage deferred. The purge runs but jemalloc decay hasn't processed the freed pages yet.

**Fix:**
```rust
pub fn flush(&self) {
    self.data.clear();
    self.reset_ttl_count();
    // Give EBR more time to advance epochs
    for _ in 0..4 {
        customhash::force_collect();
        std::thread::yield_now();
    }
    customhash::force_collect_quiescent();
    // Now purge after EBR has actually freed the memory
    super::store::purge_allocator();
}
```

### Issue 2.3: `purge()` iterates all arenas with format! allocation

**File:** `crates/rust-zmalloc/src/lib.rs` — `purge()` (~line 63)  
**Problem:** Each iteration does `format!("arena.{arena}.purge\0")` — heap allocation per arena.

**Fix:** Use `MALLCTL_ARENAS_ALL` (4096) to purge all arenas in one call:
```rust
pub fn purge() {
    refresh_epoch();
    let name = b"arena.4096.purge\0"; // MALLCTL_ARENAS_ALL
    unsafe {
        let _ = jemalloc_sys::mallctl(
            name.as_ptr().cast(), null_mut(), null_mut(), null_mut(), 0,
        );
    }
}
```

---

## Phase 3: ZSet Performance Optimization (Critical) ✅ DONE

### Issue 3.1: `range_by_score()` — O(n) filter on sorted data!

**File:** `src/storage/value.rs` — `ZSetData::range_by_score()` (~line 882)  
**Problem:** `self.entries.iter().filter(|e| e.score >= min && e.score <= max)` scans ALL entries. But entries ARE sorted by score! This should be O(log n).

**Fix:**
```rust
pub fn range_by_score(&self, min: f64, max: f64) -> &[ZEntry] {
    let start = self.entries.partition_point(|e| e.score < min);
    let end = self.entries.partition_point(|e| e.score <= max);
    &self.entries[start..end]
}
```

### Issue 3.2: `count_in_score_range()` — O(n) count on sorted data

**File:** `src/storage/value.rs` — (~line 896)  

**Fix:** Same binary search:
```rust
pub fn count_in_score_range(&self, min: f64, max: f64) -> usize {
    let start = self.entries.partition_point(|e| e.score < min);
    let end = self.entries.partition_point(|e| e.score <= max);
    end - start
}
```

### Issue 3.3: `rev_range_by_score()` — allocates Vec then reverses

**File:** `src/storage/value.rs` — (~line 889)  

**Fix:** Binary search + reverse iterator over slice (zero allocation):
```rust
pub fn rev_range_by_score(&self, min: f64, max: f64) -> impl Iterator<Item = &ZEntry> + '_ {
    let start = self.entries.partition_point(|e| e.score < min);
    let end = self.entries.partition_point(|e| e.score <= max);
    self.entries[start..end].iter().rev()
}
```

### Issue 3.4: `get_score()` — O(n) linear scan

**File:** `src/storage/value.rs` — `ZSetData::get_score()` (~line 870)  
**Problem:** `self.entries.iter().find(|e| e.member == member)` is O(n).

**Fix (zero extra memory):** Use fingerprint bloom filter (already exists!) to skip non-matches, then linear scan on bloom hits. For large sets, consider binary search on member within same-score groups. This is already reasonably fast for sets < 1000 due to cache locality.

**Fix (with index, trades memory for speed):** Add `HashMap<SmallStr, u32>` mapping member → index. Makes `get_score` O(1).

### Issue 3.5: `rank()` — O(n) linear scan

**File:** `src/storage/value.rs` — `ZSetData::rank()` (~line 876)  
**Problem:** Same linear scan as `get_score()`.

**Fix:** Same as 3.4 — with index it's O(1), without index it stays O(n).

### Issue 3.6: `remove()` — O(n) scan + O(n) shift + O(n) fingerprint rebuild

**File:** `src/storage/value.rs` — `ZSetData::remove()` (~line 856)  
**Problem:** Three O(n) passes:
1. Linear search for member
2. `Vec::remove()` shifts all elements
3. `rebuild_fingerprints()` scans all entries

**Fix:**
- Don't rebuild fingerprints on every remove. Bloom filters tolerate false positives. Track a `stale_count` and only rebuild when `stale_count > len / 4`.
- The Vec shift is unavoidable without changing the data structure.

```rust
pub fn remove(&mut self, member: &str) -> Option<f64> {
    let pos = self.entries.iter().position(|e| e.member.as_str() == member)?;
    let score = self.entries[pos].score;
    self.entries.remove(pos);
    // DON'T rebuild fingerprints — bloom tolerates stale bits
    self.reclaim_capacity();
    Some(score)
}
```

### Issue 3.7: `pop_min()` — O(n) shift + unnecessary fingerprint rebuild

**File:** `src/storage/value.rs` — `ZSetData::pop_min()` (~line 901)  
**Problem:** `self.entries.remove(0)` shifts ALL entries left. Then `rebuild_fingerprints()`.

**Fix:**
```rust
pub fn pop_min(&mut self) -> Option<ZEntry> {
    if self.entries.is_empty() { return None; }
    let entry = self.entries.remove(0);
    // No fingerprint rebuild needed — stale bits are harmless
    self.reclaim_capacity();
    Some(entry)
}
```

### Issue 3.8: `incr()` — 3× O(n)

**File:** `src/storage/value.rs` — `ZSetData::incr()` (~line 1085)  
**Problem:** Linear search + remove + insert = 3 O(n) operations.

**Fix:** Combine into single pass:
```rust
pub fn incr(&mut self, member: &str, increment: f64) -> f64 {
    if let Some(pos) = self.entries.iter().position(|e| e.member.as_str() == member) {
        let new_score = self.entries[pos].score + increment;
        // Check if position changes
        let stays = (pos == 0 || self.entries[pos-1].score <= new_score)
            && (pos == self.entries.len()-1 || self.entries[pos+1].score >= new_score);
        if stays {
            self.entries[pos].score = new_score;  // in-place update, no shift!
            return new_score;
        }
        // Only shift if position actually changes
        self.entries.remove(pos);
        let new_pos = self.find_insert_pos(new_score, member);
        self.entries.insert(new_pos, ZEntry { score: new_score, member: SmallStr::new(member) });
        new_score
    } else {
        // New member
        let insert_pos = self.find_insert_pos(increment, member);
        self.entries.insert(insert_pos, ZEntry { score: increment, member: SmallStr::new(member) });
        self.ensure_fingerprint_capacity();
        self.set_fingerprint(member_fingerprint(member));
        increment
    }
}
```

---

## Phase 4: zmalloc Improvements (Redis Parity) ✅ DONE

### Issue 4.1: No instant `used_memory()` — requires `refresh_epoch()`

**Problem:** `stats()` calls `refresh_epoch()` + multiple mallctl queries. This is slow (~1µs) and demand-faults pages.

**Fix:** Add an `AtomicUsize` counter:
```rust
use std::sync::atomic::{AtomicUsize, Ordering};

static USED_MEMORY: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Zmalloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = jemallocator::Jemalloc.alloc(layout);
        if !ptr.is_null() {
            USED_MEMORY.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        jemallocator::Jemalloc.dealloc(ptr, layout);
        USED_MEMORY.fetch_sub(layout.size(), Ordering::Relaxed);
    }
}

pub fn used_memory() -> usize {
    USED_MEMORY.load(Ordering::Relaxed)
}
```

Note: `layout.size()` is the requested size, not usable size. For exact tracking, use `jemalloc_sys::sallocx()` but that's slower. The layout-based approach is what Redis does (prefix header stores exact size).

### Issue 4.2: No fragmentation ratio

**Fix:**
```rust
pub fn fragmentation_ratio() -> f64 {
    let s = stats();
    if s.allocated == 0 { 1.0 }
    else { s.active as f64 / s.allocated as f64 }
}
```

### Issue 4.3: Reduce jemalloc idle footprint

**Problem:** jemalloc creates `4 × num_cpus` arenas by default. On a 12-thread machine = 48 arenas. Each arena has metadata that gets demand-faulted.

**Fix:** Compile-time config:
```rust
// In crates/rust-zmalloc/src/lib.rs
#[unsafe(export_name = "malloc_conf")]
#[used]
static MALLOC_CONF: &[u8] = b"narenas:2,dirty_decay_ms:200,muzzy_decay_ms:200\0";
```

Or use the same env var as Docker: `MALLOC_CONF=narenas:2,dirty_decay_ms:200,muzzy_decay_ms:200`

---

## Phase 5: Minor Memory Optimizations ✅ DONE

### Issue 5.1: `defragment_values()` clones every value

**File:** `crates/customhash/src/lib.rs` — `defragment_values()` (~line 949)  
**Problem:** `value.clone()` + `compact_allocations()` + write-back. Clones entire value.

**Fix:** Call `compact_allocations()` directly in `update_with`:
```rust
pub fn defragment_values(&self, budget: usize, mut rebuild: impl FnMut(&mut V)) -> usize {
    let keys = self.keys();
    let mut rebuilt = 0;
    for key in keys.into_iter().take(budget) {
        if self.update_with(&key, |value| rebuild(value)).is_some() {
            rebuilt += 1;
        }
    }
    rebuilt
}
```
`compact_allocations()` only calls `shrink_to_fit()` — safe in-place under the lock.

### Issue 5.2: Connection write buffer retained at 256 KB

**File:** `src/handler/conn.rs` — `RETAINED_WRITE_BUFFER = 256 * 1024`

**Fix:** Reduce to 32 KB — most responses are < 1 KB:
```rust
const RETAINED_WRITE_BUFFER: usize = 32 * 1024;
```

### Issue 5.3: `zinterstore` clones the first zset

**File:** `src/storage/zset.rs` — `zinterstore()` (~line 401)  
**Problem:** `z.clone()` copies entire first set.

**Fix:** Iterate by reference, collect members into a temporary Vec of `(&str, f64)` instead of cloning the full ZSetData.

### Issue 5.4: RespParser `rbuf` starts at 8 KB

**File:** `src/utils/parser.rs` — `RespParser::new()` (line 32)  
**Problem:** Each connection starts with `vec![0u8; 8192]`. Most commands fit in < 512 bytes.

**Fix:** Start at 2 KB, grow on demand:
```rust
rbuf: vec![0u8; 2 * 1024],
```

---

## Implementation Priority

| # | Phase | Effort | Impact | Notes |
|---|-------|--------|--------|-------|
| 1 | 1.1 | Low | High | Fix wrong RSS reporting — cosmetic but confusing |
| 2 | 3.1-3.3 | Low | High | ZSet range queries O(n)→O(log n), 3 line change each |
| 3 | 3.6-3.7 | Low | Medium | Remove fingerprint rebuild on every remove/pop |
| 4 | 2.1-2.3 | Medium | Medium | FLUSHALL memory return |
| 5 | 3.8 | Low | Medium | incr() in-place when position unchanged |
| 6 | 4.3 | Low | Low | jemalloc narenas:2 config |
| 7 | 5.1 | Low | Low | defrag without clone |
| 8 | 4.1 | Medium | Low | Atomic used_memory counter |

---

## Key Insight

**The server is NOT using 22 MB.** It's using 3.8 MB (VmRSS from kernel). The inflated number comes from:
1. jemalloc's `stats.resident` includes demand-faulted metadata pages
2. The act of READING stats causes page faults that inflate the measurement
3. `allocator_retained` (9.2 MB) is virtual address space, not physical RAM

Redis has the same "issue" but its `zmalloc_used_memory()` reports allocated bytes (not RSS), so users see the correct working set size. FyroDB already does this correctly with `used_memory:693632` — it's only the `used_memory_rss` that's misleading.
