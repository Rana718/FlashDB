# FyroDB Rust-Zmalloc Migration

## Completed

- Standalone reusable `crates/rust-zmalloc` crate.
- Process-wide `GlobalAlloc` wrapper backed by jemalloc.
- Epoch refresh before allocator statistics.
- `allocated`, `active`, `resident`, `retained`, and `muzzy` statistics API.
- Arena purge API.
- Explicit raw allocation/deallocation helpers.
- FyroDB switched from direct `jemallocator::Jemalloc` to `rust_zmalloc::Zmalloc`.

## Phase 1: Metrics Integration

- Replace FyroDB-local jemalloc mallctl code with `rust_zmalloc::stats()`.
- Add allocator categories to `INFO`.
- Add allocation/active/resident/retained regression tests.

## Phase 2: Ownership-Safe Raw Entries

### Progress

- `CustomMap::Entry` allocation now uses `rust_zmalloc::alloc_raw`.
- Entry destruction runs `Drop` and then matching `dealloc_raw` after EBR.
- Growth, delete, clear, and shard compaction entry retirement paths migrated.
- SlotTable and slot-array ownership remain on the next subphase.

### Additional Progress

- `SlotTable` object allocation now uses `rust_zmalloc::alloc_raw`.
- Growth, compaction, clear, and final destruction use typed raw-table
  destructors after EBR safety.
- Slot pointer slice storage now uses aligned `rust_zmalloc::alloc_raw` memory
  with typed `Drop` deallocation.
- All probing and iteration uses bounded slice views over the raw array.
- Added a tested no-tcache API for future isolated long-lived allocation use;
  the map remains on normal raw allocation until its concurrent update path is
  proven under stress.

- Define a typed allocation header containing layout and destructor metadata.
- Migrate `CustomMap` Entry allocation from `Box` to `alloc_raw`.
- Migrate SlotTable allocation and slot-array ownership.
- Add EBR callbacks that call `dealloc_raw` only after object destruction.
- Stress concurrent growth, clear, delete, and reads.

## Phase 3: Persistent Allocation Classes

- Use no-tcache allocation only for long-lived map entries (pending stress
  validation; current map stays on the proven raw path).
- Keep transient parser/output allocations on normal Rust allocation paths.
- Added alignment/content and allocator-stat regression tests.
- Measure fragmentation and latency before/after on the production workload.

## Phase 4: Active Defragmentation

- Add opt-in maintenance pass based on `active / allocated` fragmentation.
- Rebuild sparse shard tables under the existing growth gate.
- Relocate only EBR-safe entries.
- Never move entries while a pinned reader can access the old address.

## Phase 5: Collection Allocations

- Evaluate explicit allocation for ZSet vectors/fingerprints.
- Evaluate Hash/Set/List backing storage.
- Preserve compact numeric-set and demotion behavior.

## Safety Requirements

- Never call `dealloc_raw` before EBR grace period completion.
- Never mix `Box::from_raw` with `alloc_raw` pointers.
- Every raw allocation must have exactly one matching free.
- Run `cargo test --release --tests`, CustomMap concurrency tests, and RSS audits.
