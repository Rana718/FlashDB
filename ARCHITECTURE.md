# FlashDB — Architecture

## Overview

FlashDB is a Redis-compatible in-memory key-value store written in Rust. It speaks the RESP wire protocol so any Redis client works out of the box. It uses a thread-per-core event loop architecture, a fully lock-free concurrent hash map with epoch-based reclamation, zero-copy RESP parsing, and direct-write response building.

---

## Benchmark Results (12-core machine, Intel i5-11400H)

| Metric           | FlashDB (12 cores) | Redis (1 core) | vs Redis |
| ---------------- | ------------------ | -------------- | -------- |
| Sequential SET   | ~860k ops/sec      | ~135k ops/sec  | 6.4x     |
| Pipelined SET    | ~9.7M ops/sec      | ~1.19M ops/sec | 8.2x     |
| Pipelined GET    | ~10.9M ops/sec     | ~1.68M ops/sec | 6.5x     |
| Pub/Sub delivery | ~12M msg/sec       | ~1.06M msg/sec | 11.3x    |

Internal store throughput (no TCP):

| Operation     | Throughput    |
| ------------- | ------------- |
| SET (new key) | 18.6M ops/sec |
| SET (update)  | 29.8M ops/sec |
| GET           | 34.3M ops/sec |

---

## Why It's Fast

| Factor            | Redis                    | FlashDB                                               |
| ----------------- | ------------------------ | ----------------------------------------------------- |
| I/O model         | Single-thread epoll      | Thread-per-core epoll (mio), SO_REUSEPORT             |
| Accept queue      | Single shared queue      | Per-thread kernel queue — no contention               |
| Hash map          | Custom, single-threaded  | Lock-free CustomMap — EBR, atomic swap, value pooling |
| RESP parsing      | Copy into dynamic buffer | Zero-copy: parse directly from read buffer            |
| Command dispatch  | String comparison        | Zero-alloc byte comparison, inline SET/GET fast path  |
| Response building | Format into String       | Write raw bytes directly into write buffer            |
| GET path          | Clone value + allocate   | Zero-copy: write directly from stored value to buffer |
| Allocator         | libc malloc              | mimalloc — faster small allocation                    |
| Memory search     | Naive byte scan          | SIMD memchr (AVX2) for newline scanning               |
| Write batching    | Per-command write        | Batched: all events read first, then flush all writes |

---

## CustomMap — Lock-Free Concurrent Hash Map

FlashDB uses a custom-built fully lock-free sharded hash map (`crates/customhash/`). No mutex, no RwLock, no spinlock anywhere on the data path.

### Design

```
CustomMap<V>
  ├── shards: Box<[Shard<V>]>    (num_cpus × 32, power of two)
  ├── hasher: foldhash RandomState
  └── key_count: AtomicUsize

Shard<V>
  ├── slots: Box<[AtomicPtr<Entry<V>>]>   (fixed capacity, linear probe)
  ├── mask: usize
  ├── len: AtomicUsize (occupancy)
  └── threshold: usize (70% load factor)

Entry<V>
  ├── hash: u64
  ├── key: String               (immutable after publish)
  └── value: AtomicPtr<ValueBox<V>>  (swapped atomically on update)
```

### Lock-Free Invariants

1. **Entry slots are write-once** — a slot transitions `null → Entry*` exactly once via CAS, never back. No ABA problem, no slot reuse. Readers probe slots freely without pinning.

2. **Values are swapped atomically** — updates replace the value pointer with one `swap`. The old `ValueBox` is retired through epoch-based reclamation. Readers holding the old pointer are protected by their epoch pin.

3. **Slot array never reallocates** — fixed capacity, sized at construction. No pointer invalidation, no resize locking.

### Epoch-Based Reclamation (EBR)

```
Thread-Local:
  ├── participant: Participant* (in global linked list)
  ├── garbage: Vec<Garbage>     (retired pointers)
  ├── pool: Vec<*mut ValueBox>  (recycled allocations)
  ├── depth: usize              (pin nesting)
  └── retires: usize            (collection trigger)

Global:
  ├── GLOBAL_EPOCH: AtomicU64
  └── PARTICIPANTS: lock-free linked list
```

**Grace period**: a retired pointer is safe to free once the global epoch has advanced by 2 beyond its retirement epoch. This guarantees every reader that could have loaded the pointer has since unpinned.

**Value pooling**: reclaimed `ValueBox` allocations are kept in a thread-local pool (up to 1024). On SET (update path), the pool provides a pre-allocated box — avoiding malloc entirely for steady-state workloads.

**Single TLS access per operation**: the SET hot path (`replace_value`) does alloc + swap + retire in one `thread_local!` access. The GET hot path (`read_clone`) does pin + load + clone + unpin in one access. This eliminates the overhead of repeated TLS lookups.

### Performance Characteristics

| Operation               | Mechanism                                 | Cost   |
| ----------------------- | ----------------------------------------- | ------ |
| Read (find)             | Wait-free linear probe, Acquire loads     | ~30ns  |
| Read (get value)        | Pin + load + clone + unpin (one TLS)      | ~50ns  |
| Write (update existing) | Alloc from pool + swap + retire (one TLS) | ~60ns  |
| Write (new key)         | CAS on len + CAS on slot + alloc entry    | ~120ns |
| Remove                  | Swap to null + retire                     | ~50ns  |
| Contains                | Wait-free, no pin needed                  | ~25ns  |

### Why Not DashMap

| Aspect                | DashMap                    | CustomMap                               |
| --------------------- | -------------------------- | --------------------------------------- |
| Read contention       | RwLock per shard           | No lock at all                          |
| Write contention      | Exclusive lock per shard   | CAS on single slot                      |
| Memory reclamation    | Immediate (under lock)     | Deferred (EBR, amortized)               |
| Value access          | Returns guard (holds lock) | Returns zero-copy ref (holds epoch pin) |
| Throughput (12 cores) | ~7.5M SET/s                | ~9.7M SET/s                             |
| Internal (no TCP)     | ~20M SET/s                 | ~30M SET/s                              |

---

## Source Layout

```
src/
├── main.rs                   Entry point: signal mask, thread spawn, RDB load
├── lib.rs                    Module declarations
├── worker.rs                 Per-thread epoll loop, batched write pass
│
├── handler/
│   ├── conn.rs               Conn struct, do_read/do_write, inline SET/GET fast path
│   ├── dispatch.rs           Command routing (SET/GET short-circuit, then enum dispatch)
│   ├── subscription.rs       SUBSCRIBE/UNSUBSCRIBE handling
│   └── pubsub_cmds.rs        PUBSUB CHANNELS/NUMSUB/NUMPAT
│
├── commends/
│   ├── mod.rs                ComdType enum + execute() dispatcher
│   ├── connection.rs         PING, ECHO, INFO, FLUSH, DBSIZE, TYPE, BGSAVE
│   ├── string.rs             All string commands (SET, GET, INCR, etc.)
│   ├── keys.rs               Key management (DEL, EXPIRE, RENAME, COPY, etc.)
│   ├── hash.rs               Hash commands (HSET, HGET, HGETALL, etc.)
│   └── scan.rs               SCAN cursor iteration
│
├── storage/
│   ├── store.rs              Store struct — Arc<CustomMap<StoreValue>>
│   ├── value.rs              FlashDB enum + StoreValue + TTL + time utilities
│   ├── string.rs             String storage ops (set_string, get, get_to_buf, incr, etc.)
│   ├── hash.rs               Hash storage ops (lock-free read-modify-write via try_update)
│   ├── keys.rs               Key ops (del, expire, rename, copy, etc.)
│   ├── scan.rs               SCAN implementation
│   ├── server.rs             INFO, FLUSH, cleanup_expired
│   └── rdb.rs                RDB save/load/background save
│
├── pubsub/
│   ├── registry.rs           Sharded RwLock channel map (64 shards)
│   ├── slot.rs               SubSlot (SegQueue + Waker notify)
│   └── frame.rs              RESP message encoding
│
├── macros/
│   ├── cmd.rs                parse_int!, parse_float!, wt!, store_ok!
│   ├── string_enum.rs        Zero-alloc command enum macro
│   └── sub.rs                Pub/sub reply macros
│
└── utils/
    ├── parser.rs             Zero-copy RESP parser (SIMD memchr)
    ├── resp.rs               Raw byte response builders
    └── util.rs               glob_match, format_float

crates/customhash/src/
├── lib.rs                    CustomMap<V> — lock-free sharded hash map
└── ebr.rs                    Epoch-based reclamation with value pooling
```

---

## Request Lifecycle

```
Client (redis-cli / any RESP client)
    │
    │  TCP  (TCP_NODELAY, SO_REUSEPORT)
    ▼
Per-thread TcpListener                         [worker.rs]
    │  Kernel distributes connections across threads
    ▼
mio epoll event loop                           [worker.rs]
    │  Read pass: process all ready events
    │  Write pass: flush all dirty connections (batched)
    ▼
Conn::do_read()                                [handler/conn.rs]
    │  drain socket → parse commands in tight loop
    ▼
Inline fast path (SET/GET)                     [handler/conn.rs]
    │  Check first 3 bytes of command directly from raw pointers
    │  SET: store.set_string(key, value, 0) → write "+OK\r\n"
    │  GET: store.get_to_buf(key, wbuf) → zero-copy write
    │  No &str array construction, no enum dispatch
    ▼
General dispatch (other commands)              [handler/dispatch.rs → commends/]
    │  Build &str array on stack → ComdType::from_bytes() → handler
    ▼
Store operation                                [storage/]
    │  CustomMap: hash → shard → linear probe → atomic op
    ▼
Batched write flush                            [worker.rs]
    │  All dirty connections written in one pass
    ▼
Client receives response
```

---

## Data Model

```
CustomMap<StoreValue>           Lock-free, O(1) avg
         │
         └── StoreValue (32 bytes)
                 ├── value: FlashDB (24 bytes)
                 │       ├── String(String)         — 24 bytes inline
                 │       └── Hash(Box<HashMap>)     — 8 byte pointer
                 └── expires_ms: u64               — 0 = no expiry
```

**Expiry strategy:**

- **Lazy**: checked on every access — expired keys return None. Zero background cost.
- **Active**: background thread calls `retain()` every second. Removes unreachable expired keys. O(n) but off hot path.

---

## Pub/Sub Architecture

```
Publisher thread (any worker)
    │  PUBLISH channel message
    ▼
PubSub::publish()                              [pubsub/registry.rs]
    │  Shard lookup (64 RwLock shards, read-lock only on publish)
    │  Encode frame once as Arc<[u8]>
    │  Push Arc clone to each subscriber's SegQueue
    ▼
SubSlot::push()                                [pubsub/slot.rs]
    │  SegQueue::push (lock-free)
    │  AtomicBool notify coalescing (one wake per batch)
    │  WorkerNotifier → Waker::wake() (eventfd)
    ▼
Subscriber's worker thread
    │  WAKER_TOKEN event in epoll
    │  drain_into(wbuf) — copy all queued messages to write buffer
    │  do_write() — flush to subscriber socket
    ▼
Subscriber receives messages
```

**Key design decisions:**

- Frame encoded once, shared via `Arc<[u8]>` — zero-copy fan-out
- SegQueue (lock-free MPMC) per subscriber — publishers never block
- Notify coalescing — one `wake()` syscall per drain cycle, not per message

---

## Concurrency Model

```
main thread
    ├── worker 0..N: epoll loop (one per CPU core)
    ├── cleanup thread: retain() every 1s
    └── rdb thread: save() every 300s

All workers share:
    ├── Arc<Store> → Arc<CustomMap<StoreValue>>
    └── Arc<PubSub> → sharded RwLock channel registry
```

**No global lock** on the hot path. Two clients writing to different keys never contend. Two clients writing to the same key contend only on a single atomic swap (lock-free).

---

## Persistence — RDB Snapshots

Format: `FLDB` magic + version byte + entries + `0xFF` EOF.

Each entry: `type(1) + ttl_ms(8) + key_len(4) + key + type-specific payload`

- Atomic write: `tmp` file → `rename()` — crash-safe
- 1MB buffered I/O
- Expired keys skipped on load
- Triggers: startup load, periodic (300s), BGSAVE command, SIGTERM/SIGINT

---

## Complexity Reference

| Operation          | Time     | Notes                                      |
| ------------------ | -------- | ------------------------------------------ |
| GET / EXISTS / TTL | O(1) avg | Lock-free probe + atomic load              |
| SET / DEL / EXPIRE | O(1) avg | Lock-free probe + atomic swap              |
| INCR / APPEND      | O(1) avg | Read-modify-write via try_update           |
| HGET / HSET / HDEL | O(1) avg | Outer probe + inner HashMap clone-and-swap |
| HGETALL            | O(f)     | f = number of fields                       |
| MGET / MSET        | O(k)     | k = number of keys                         |
| KEYS pattern       | O(n)     | Full scan + glob match                     |
| SCAN cursor        | O(count) | Sorted key slice, stable cursor            |
| PUBLISH            | O(s)     | s = subscribers on channel                 |
| cleanup_expired    | O(n)     | retain() scan, runs off hot path           |
| RDB save/load      | O(n)     | Sequential scan + buffered I/O             |
| RESP parse         | O(b/32)  | b = bytes, SIMD memchr                     |
