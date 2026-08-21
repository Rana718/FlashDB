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
| Command dispatch | String comparison | First-byte fast-path for 13 hot commands; static `foldhash` map (O(1)) for all others |
| Response building | Format into String | Inline bulk headers, raw byte writes |
| GET path | Clone value + allocate | Zero-copy: write directly from stored value to buffer |
| Mutations | Single-threaded (safe) | In-place under per-key spinlock, seqlock for reader safety |
| TTL computation | clock_gettime per op | Cached clock ticked once per event loop iteration |
| Allocator | libc malloc | mimalloc with zero-overhead hot path and periodic RSS tracking |
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

Shard<V>  (cache-padded counters)
  ├── table: AtomicPtr<SlotTable<V>>     (swapped atomically on growth)
  ├── len: CachePadded<AtomicUsize>      (occupied slots)
  ├── insert_gate: CachePadded<AtomicUsize>  (concurrent insert counter + GROWING flag)
  └── grow_lock: Mutex<()>               (serializes growth/compaction per shard)

Entry<V>  (per key, heap-allocated)
  ├── hash: u64                          (full 64-bit hash, cached)
  ├── key: CompactKey                    (≤15 bytes inline)
  ├── state: AtomicU64                   (lock + occupied bit + seqlock generation)
  └── value: UnsafeCell<MaybeUninit<V>>  (mutated in place while occupied)
```

### Concurrency Model

**Writers** (SET, LPUSH, SADD, HSET, ZADD, INCR):
```
1. Find entry via lock-free linear probe
2. Acquire per-key spinlock (single atomic CAS, Acquire ordering)
3. Increment the packed sequence to odd (signals "write in progress")
4. Mutate value in-place (zero clone, zero allocation)
5. Store final state: increment seq to even + release lock (single atomic store)
```

**Single-field Readers** (GET, HGET, SISMEMBER, ZSCORE, LINDEX):
```
1. Find entry via lock-free linear probe
2. Pin EBR epoch (thread-local atomic store)
3. Validate the occupied bit (Acquire ordering)
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

Every shard starts with eight slots and grows only as keys arrive. When a shard reaches 90% occupancy:
1. Set GROWING flag in insert_gate (blocks new inserts)
2. Wait for in-flight inserts to complete
3. Allocate new SlotTable at 2× capacity
4. Copy live Entry pointers (skip tombstones)
5. Atomic swap of table pointer
6. Retire old table via EBR
7. Clear GROWING flag

### Epoch-Based Reclamation (EBR)

When an entry or slot table is retired:
- Stamped with current global epoch
- Added to thread-local garbage list
- Freed only after all threads have advanced past epoch + 2
- Raw entry/table memory is released through the tracked allocator

Updates to existing keys mutate their value in place. Allocations are needed only when the new value or collection representation itself grows.

### Compact Values

- `SmallStr` stores strings up to 23 bytes inline
- Small hashes and lists use compact sequential storage
- Sets use integer, compact-vector, or full hash-set representations and promote/demote with size
- Sorted sets use one score-ordered `Vec<ZEntry>` with a bloom filter for fast negative member lookups; score-range operations use binary partition points
- Background maintenance can shrink collection capacity and rebuild fragmented values under the existing entry lock

### Memory Maintenance

- EBR and allocator collection run every 10 seconds
- Fragmentation checks and bounded value defragmentation run every 60 seconds
- Underutilized shard tables are compacted every 120 seconds
- Flush performs repeated EBR collection, a quiescent collection, then allocator purge

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
│   ├── value/           FyroDB values, SmallStr, compact collections, JSON, ZSetData
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
├── lib.rs               Public CustomMap API
├── shard.rs             Entry/table layout, probing, growth
├── ops.rs               iteration, clear, compaction, defragmentation
├── key.rs               15-byte inline CompactKey
└── ebr.rs               Epoch-based reclamation

crates/rust-zmalloc/src/
└── lib.rs               mimalloc allocator, RSS stats, purge (mi_collect)
```

---

## Complexity Reference

| Operation | Time | Mechanism |
| --------- | ---- | --------- |
| GET / HGET / SISMEMBER | O(1) | Lock-free probe + atomic load |
| SET / DEL / EXPIRE | O(1) average | Lock-free probe + per-entry mutation/removal |
| INCR / LPUSH / SADD | O(1) | Per-key spinlock + in-place mutate |
| HGETALL / SMEMBERS | O(N) | Seqlock-validated iteration |
| ZADD | O(N) worst case | Bloom filter skips scan for new members; append is O(1) for ascending scores |
| ZRANGE | O(K) | Contiguous sorted Vec slice |
| ZRANGEBYSCORE | O(log N + K) | Binary partition points + slice iteration |
| ZRANK / ZSCORE | O(N) | Linear member lookup |
| ZPOPMIN / ZPOPMAX | O(N) / O(1) | Vec front removal / tail pop |
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
| Command dispatch (fast-path) | O(1) | First-byte + `cmd_eq` for 13 hot commands (GET/SET/HGET/HSET/LPUSH/LPOP/LRANGE/RPUSH/RPOP/EXPIRE/ZADD/JSON.GET/JSON.SET) |
| Command dispatch (all others) | O(1) | Static `OnceLock<foldhash::HashMap>` — uppercase to 32-byte stack buf + one hash probe; built once, zero allocation per call |
| Hash probe (avg) | O(1) | Open addressing, 90% load factor |
| Hash probe (worst) | O(N/S) | N = keys in shard, linear probe |
| Growth / Resize | O(N/S) | Per-shard, copies live pointers only |
| EBR collect | O(G) | G = garbage list length |

### Data Structures Used

| Type | Structure | Why |
| ---- | --------- | --- |
| Key→Value mapping | Open-addressing hash table (linear probe) | Cache-friendly, no pointer chasing |
| Hash fields | compact `Vec<SmallStr>` → `foldhash::HashMap<SmallStr, SmallStr>` | Low overhead for small hashes, O(1) access after promotion |
| List | compact/full `VecDeque<SmallStr>` | Inline short values and O(1) push/pop both ends |
| Set | sorted integers → compact `Vec<SmallStr>` → `foldhash::HashSet<SmallStr>` | Representation follows member type and cardinality |
| Sorted Set | score-ordered `Vec<ZEntry>` + bloom filter | Compact memory, O(1) append for ascending scores, O(log n) score ranges, bloom skips scan for new members |
| Stream consumer groups | `foldhash::HashMap<String, ConsumerGroup>` | O(1) group/consumer lookup |
| Command name → enum | `OnceLock<foldhash::HashMap<&'static str, ComdType>>` | O(1) dispatch after one-time init |
| JSON | Custom recursive enum (`JsonValue`) | Zero-dependency, path traversal |
| Stream | `BTreeMap<StreamId, Vec<(String, String)>>` | Ordered by ID, O(log N) range |
| HyperLogLog | 16384-register byte array | Fixed 16KB, probabilistic counting |
| Geospatial | ZSet with geohash-encoded scores | Reuses sorted set, haversine filtering |
| Pub/Sub channels | Arc-snapshot Vec per shard | Lock-free publish, copy-on-write subscribe |
| Subscriber queue | `crossbeam::SegQueue` | Lock-free MPMC, bounded backpressure |
| EBR garbage | Thread-local `Vec<Garbage>` | Batched collection every 512 retires |
| Allocator accounting | `rust-zmalloc` + mimalloc | Zero-overhead allocator; RSS tracked periodically via /proc, explicit page release via mi_collect |
| Shard selection | Bit shift + mask (foldhash) | Single instruction, no modulo |
| RESP newline scan | `memchr` (SIMD AVX2) | 32 bytes per cycle |
