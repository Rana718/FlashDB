# FyroDB — Architecture

## Overview

FyroDB is a Redis-compatible in-memory key-value store written in Rust. It speaks the RESP wire protocol so any Redis client works out of the box. It uses a thread-per-core event loop architecture, a lock-free concurrent hash map with epoch-based reclamation and per-key seqlock, zero-copy RESP parsing, and direct-write response building.

---

## Why It's Fast

| Factor | Redis | FyroDB |
| ------ | ----- | ------ |
| I/O model | Single-thread epoll | Thread-per-core epoll (mio), SO_REUSEPORT |
| Accept queue | Single shared queue | Per-thread kernel queue — no contention |
| Hash map | Custom, single-threaded | Lock-free CustomMap — EBR + seqlock + per-key spinlock |
| RESP parsing | Copy into dynamic buffer | Zero-copy: parse directly from read buffer |
| Command dispatch | String comparison | First-byte fast-path for hot commands, length-gated enum fallback |
| Response building | Format into String | Inline bulk headers, raw byte writes |
| GET path | Clone value + allocate | Zero-copy: write directly from stored value to buffer |
| Mutations | Single-threaded (safe) | In-place under per-key spinlock, seqlock for reader safety |
| TTL computation | clock_gettime per op | Cached clock ticked once per event loop iteration |
| Allocator | libc malloc | mimalloc — faster small allocation |
| Memory search | Naive byte scan | SIMD memchr (AVX2) for newline scanning |
| Write batching | Per-command write | Batched: all events read first, then flush all writes |
| Pub/Sub fan-out | Per-message frame copy | Single Arc allocation, shared to all subscribers |

---

## CustomMap — Lock-Free Concurrent Hash Map

### Memory Layout

```
CustomMap<V>
  ├── shards: Box<[Shard<V>]>           (N shards, power of two)
  ├── shift / shard_mask                 (fast shard selection via hash >> shift)
  ├── hasher: foldhash::RandomState      (non-cryptographic, fast hash)
  ├── key_count: AtomicUsize             (global live key count)
  └── max_keys: usize                    (capacity limit)

Shard<V>  (128-byte aligned — no false sharing)
  ├── table: AtomicPtr<SlotTable<V>>     (swapped atomically on growth)
  ├── len: CachePadded<AtomicUsize>      (occupied slots)
  ├── insert_gate: CachePadded<AtomicUsize>  (concurrent insert counter + GROWING flag)
  └── grow_lock: Mutex<()>               (serializes growth only)

Entry<V>  (per key, heap-allocated)
  ├── hash: u64                          (full 64-bit hash, cached)
  ├── key: String                        (immutable after insertion)
  ├── value: AtomicPtr<ValueBox<V>>      (swapped atomically on SET)
  ├── mlock: AtomicU8                    (per-key spinlock for writers)
  └── seq: AtomicU32                     (seqlock counter for reader safety)
```

### Concurrency Model

**Writers** (SET, LPUSH, SADD, HSET, ZADD, INCR):
```
1. Find entry via lock-free linear probe
2. Acquire per-key spinlock (single atomic swap, nanoseconds)
3. Increment seq to odd (signals "write in progress")
4. Mutate value in-place (zero clone, zero allocation)
5. Increment seq to even (signals "write complete")
6. Release spinlock
```

**Single-field Readers** (GET, HGET, SISMEMBER, ZSCORE, LINDEX):
```
1. Find entry via lock-free linear probe
2. Pin EBR epoch (thread-local atomic store)
3. Load value pointer (Acquire ordering)
4. Read field directly — no lock, no seq check
5. Unpin epoch
```

**Iteration Readers** (LRANGE, SMEMBERS, HGETALL, HKEYS, HVALS):
```
1. Find entry via lock-free linear probe
2. Pin EBR epoch
3. Read seq counter (must be even — wait if odd)
4. Iterate collection, clone results
5. Read seq counter again
6. If seq changed → retry from step 3 (writer raced)
7. If seq unchanged → return results (consistent snapshot)
8. Unpin epoch
```

### Safety Guarantees

- **Writer vs Writer**: per-key spinlock serializes — no two mutations on same key simultaneously
- **Reader vs Writer (single field)**: EBR keeps value alive, single lookup is atomic
- **Reader vs Writer (iteration)**: seqlock detects race, reader retries if write happened during iteration
- **Different keys**: zero contention — separate entries, separate spinlocks
- **No deadlock**: spinlock is per-entry, held for microseconds, no nesting
- **No starvation**: readers retry at most once per concurrent write

### Dynamic Growth

When a shard reaches 70% occupancy:
1. Set GROWING flag in insert_gate (blocks new inserts)
2. Wait for in-flight inserts to complete
3. Allocate new SlotTable at 2× capacity
4. Copy live Entry pointers (skip tombstones)
5. Atomic swap of table pointer
6. Retire old table via EBR
7. Clear GROWING flag

### Epoch-Based Reclamation (EBR)

When a value pointer is retired:
- Stamped with current global epoch
- Added to thread-local garbage list
- Freed only after all threads have advanced past epoch + 2
- ValueBox allocations recycled in thread-local pool (up to 1024)

Steady-state SET operations allocate zero memory — ValueBoxes recycled from pool.

---

## Pub/Sub — Lock-Free Arc Snapshot

### Publish Path (Zero Locks)

```
PUBLISH channel message:
  1. Hash channel → select shard
  2. Arc::clone snapshot (single atomic increment)
  3. Find channel in snapshot, encode message frame
  4. Push Arc<[u8]> frame to each subscriber's lock-free queue
  5. Coalesced epoll wake (deduplicated)
```

### Subscribe/Unsubscribe (Copy-on-Write)

Rebuilds the channel list under a brief mutex, then atomically swaps the Arc snapshot pointer. Publishers holding the old snapshot keep it alive until they finish.

---

## Request Lifecycle

```
Client → TCP (SO_REUSEPORT) → Per-thread epoll → Conn::do_read()
  → Zero-copy RESP parse → Inline fast path (SET/GET/INCR/DEL/LPUSH/RPOP/SADD)
  → Or: dispatch table → Storage operation → Response to write buffer
  → Conn::do_write() → Client
```

All I/O is batched: read all ready events, then flush all responses in one pass.

---

## Persistence — RDB Snapshots

- Format: `FLDB` magic + version + typed entries + EOF marker
- Supports: String, Hash, List, Set, ZSet, JSON, Stream
- Atomic write: temp file → fsync → rename
- Per-slot EBR pin during save (no multi-second GC stalls)
- Expired keys skipped during load
- Truncation-safe: loads partial files with warning

---

## Source Layout

```
src/
├── main.rs              Entry point, config, signal handling
├── worker.rs            Per-thread epoll loop, batched I/O
├── handler/
│   ├── conn.rs          Connection state, inline SET/GET/INCR/DEL/LPUSH/RPOP/SADD
│   ├── dispatch.rs      First-byte fast-path + enum fallback
│   ├── subscription.rs  Pub/Sub state machine
│   └── pubsub_cmds.rs   PUBSUB subcommands
├── commends/
│   ├── mod.rs           Command enum + dispatcher
│   ├── string.rs        String commands
│   ├── hash.rs          Hash commands
│   ├── list.rs          List commands
│   ├── set.rs           Set commands
│   ├── zset.rs          Sorted Set commands
│   ├── json.rs          JSON commands
│   ├── stream.rs        Stream commands
│   ├── bitmap.rs        Bitmap commands
│   ├── hll.rs           HyperLogLog commands
│   ├── geo.rs           Geospatial commands
│   ├── keys.rs          Key management commands
│   ├── scan.rs          SCAN cursor implementation
│   ├── connection.rs    Server/connection commands
│   └── transaction.rs   MULTI/EXEC/DISCARD
├── storage/
│   ├── store.rs         Store struct (CustomMap + counters)
│   ├── value.rs         FyroDB enum, ZSetData, JsonValue, seqlock types
│   ├── string.rs        String storage operations
│   ├── hash.rs          Hash storage operations
│   ├── list.rs          List storage operations
│   ├── set.rs           Set storage operations
│   ├── zset.rs          Sorted Set storage operations
│   ├── json.rs          JSON storage operations
│   ├── stream.rs        Stream storage operations
│   ├── bitmap.rs        Bitmap storage operations
│   ├── hll.rs           HyperLogLog storage operations
│   ├── geo.rs           Geospatial storage operations
│   ├── keys.rs          Key operations (expire, rename, copy)
│   ├── scan.rs          Cursor-based scan
│   ├── server.rs        Info, flush, cleanup_expired
│   └── rdb.rs           RDB persistence
├── pubsub/
│   ├── registry.rs      Arc-snapshot pub/sub registry
│   ├── slot.rs          Per-subscriber lock-free queue
│   └── frame.rs         RESP message encoding
└── utils/
    ├── parser.rs        Zero-copy RESP parser (SIMD memchr)
    ├── resp.rs          RESP response builders
    ├── resp3.rs         RESP3 protocol types
    └── util.rs          Glob matching, float formatting

crates/customhash/src/
├── lib.rs               Lock-free sharded hash map with per-key seqlock
└── ebr.rs               Epoch-based reclamation with value pooling
```

---

## Complexity Reference

| Operation | Time | Mechanism |
| --------- | ---- | --------- |
| GET / HGET / SISMEMBER | O(1) | Lock-free probe + atomic load |
| SET / DEL / EXPIRE | O(1) | Lock-free probe + atomic swap |
| INCR / LPUSH / SADD | O(1) | Per-key spinlock + in-place mutate |
| HGETALL / SMEMBERS | O(N) | Seqlock-validated iteration |
| ZADD | O(log N) | BTreeMap insert under spinlock |
| ZRANGE | O(log N + K) | BTreeMap range iteration |
| ZRANK | O(N) | BTreeMap range count |
| ZPOPMIN / ZPOPMAX | O(log N) | BTreeMap first/last removal |
| LINDEX | O(N) | VecDeque index access |
| LRANGE | O(N) | Seqlock + VecDeque slice iteration |
| LREM / LPOS | O(N) | Linear scan of VecDeque |
| LPUSH / RPUSH | O(1) | VecDeque push_front/push_back under spinlock |
| LPOP / RPOP | O(1) | VecDeque pop_front/pop_back under spinlock |
| SINTER | O(N × M) | HashSet intersection (smallest-first) |
| SUNION / SDIFF | O(N) | HashSet union/difference |
| GEOSEARCH | O(N) | Full ZSet scan with haversine filter |
| JSON.SET (root) | O(V) | JSON parse + atomic store |
| JSON.SET (path) | O(D) | D = path depth traversal |
| JSON.GET | O(1) | Direct path lookup |
| XADD | O(1) | BTreeMap append (auto-incrementing ID) |
| XRANGE | O(log N + K) | BTreeMap range query |
| BITCOUNT | O(N) | Byte-level popcount |
| PFADD / PFCOUNT | O(1) | HyperLogLog register update/estimate |
| PUBLISH | O(S) | S = subscriber count, lock-free |
| SCAN | O(COUNT) | Hash-based cursor, stable across mutations |
| KEYS pattern | O(N) | Full scan with per-slot EBR pin |
| SORT | O(N log N) | Vec collect + sort |
| RDB save | O(N) | Per-slot iteration, buffered I/O |
| RESP parse | O(B) | B = bytes, SIMD memchr for newlines |
| Hash probe (avg) | O(1) | Open addressing, 70% load factor |
| Hash probe (worst) | O(N/S) | N = keys in shard, linear probe |
| Growth / Resize | O(N/S) | Per-shard, copies live pointers only |
| EBR collect | O(G) | G = garbage list length |

### Data Structures Used

| Type | Structure | Why |
| ---- | --------- | --- |
| Key→Value mapping | Open-addressing hash table (linear probe) | Cache-friendly, no pointer chasing |
| Hash fields | `HashMap<String, String>` | O(1) field access |
| List | `VecDeque<String>` | O(1) push/pop both ends |
| Set | `HashSet<String>` | O(1) membership test |
| Sorted Set scores | `HashMap<String, f64>` | O(1) score lookup |
| Sorted Set ordering | `BTreeMap<ScoreKey, ()>` | O(log N) range queries, ordered iteration |
| JSON | Custom recursive enum (`JsonValue`) | Zero-dependency, path traversal |
| Stream | `BTreeMap<StreamId, Vec<(String, String)>>` | Ordered by ID, O(log N) range |
| HyperLogLog | 16384-register byte array | Fixed 16KB, probabilistic counting |
| Geospatial | ZSet with geohash-encoded scores | Reuses sorted set, haversine filtering |
| Pub/Sub channels | Arc-snapshot Vec per shard | Lock-free publish, copy-on-write subscribe |
| Subscriber queue | `crossbeam::SegQueue` | Lock-free MPMC, bounded backpressure |
| EBR garbage | Thread-local `Vec<Garbage>` | Batched collection every 512 retires |
| Value pool | Thread-local `Vec<*mut ValueBox>` | Recycled allocations, zero malloc steady-state |
| Shard selection | Bit shift + mask (foldhash) | Single instruction, no modulo |
| RESP newline scan | `memchr` (SIMD AVX2) | 32 bytes per cycle |
