# FlashDB — Architecture

## Overview

FlashDB is a Redis-compatible in-memory key-value store written in Rust. It speaks the RESP wire protocol so any Redis client works out of the box. It uses a thread-per-core event loop architecture, a fully lock-free concurrent hash map with epoch-based reclamation, zero-copy RESP parsing, and direct-write response building.

---

## Benchmark Results (6-core machine, Intel i5-11400H, 12 threads)

| Metric           | FlashDB (6 cores) | Redis Cluster (6 nodes) | vs Cluster |
| ---------------- | ----------------- | ----------------------- | ---------- |
| Sequential SET   | ~15.4M ops/sec    | ~3.5M ops/sec           | 4.4x       |
| Pipelined SET    | ~15.9M ops/sec    | ~7.9M ops/sec           | 2.0x       |
| Pipelined GET    | ~19.6M ops/sec    | ~8.3M ops/sec           | 2.4x       |
| Pub/Sub delivery | ~25.66M msg/sec   | ~7.3M msg/sec           | 3.5x       |

### Resource Usage

| State          | RSS Memory | CPU Usage   |
| -------------- | ---------- | ----------- |
| Idle (no keys) | ~57 MB     | 0%          |
| Under load     | ~270 MB    | ~53% avg    |
| Peak           | ~340 MB    | ~71% peak   |

### Internal Store Throughput (no TCP overhead)

| Operation     | Throughput    |
| ------------- | ------------- |
| SET (new key) | 24.6M ops/sec |
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
| Pub/Sub fan-out   | Per-message frame copy   | Single frame allocation, Arc-shared to all subs       |
| Pub/Sub dispatch  | Single-thread loop       | Lock-free snapshot: zero locks on publish path        |

---

## CustomMap — Lock-Free Concurrent Hash Map

FlashDB uses a custom-built fully lock-free sharded open-addressing hash map (`crates/customhash/`). No mutex, no RwLock, no spinlock anywhere on the data path.

### Memory Layout

```
CustomMap<V>
  ├── shards: Box<[Shard<V>]>        (N shards, power of two, N = workers × 4)
  ├── shift: u32                      (for fast shard selection: hash >> shift)
  ├── shard_mask: usize               (N - 1)
  ├── hasher: foldhash::RandomState   (non-cryptographic, fast hash)
  └── key_count: AtomicUsize          (global live key count)

Shard<V>  (128-byte aligned, eliminates false sharing between shards)
  ├── slots: Box<[AtomicPtr<Entry<V>>]>   (fixed-size array, open addressing)
  ├── mask: usize                          (capacity - 1, for fast modulo)
  ├── len: CachePadded<AtomicUsize>        (live entries in this shard)
  └── threshold: usize                     (capacity × 70% = max occupancy)

Entry<V>  (heap-allocated, pointed to by slot)
  ├── hash: u64                      (full 64-bit hash, cached for fast compare)
  ├── key: String                    (immutable after insertion, never changes)
  └── value: AtomicPtr<ValueBox<V>>  (the only mutable field — swapped atomically)
```

### How Operations Work

**Shard Selection (O(1)):**
```
hash = foldhash(key)            // fast non-crypto hash
shard_idx = (hash >> shift) & shard_mask   // upper bits select shard
```
Upper bits are used because they have better distribution in open addressing (lower bits determine probe position within the shard).

**GET — Lock-Free Read:**
```
1. hash(key) → select shard
2. Linear probe from (hash & mask):
     load AtomicPtr<Entry> at slot[i]
     if null → key doesn't exist, return None
     if entry.hash == hash && entry.key == key → found
     else i = (i + 1) & mask (next slot)
3. Pin EBR epoch (single TLS access)
4. Load entry.value (AtomicPtr, Acquire ordering)
5. Clone the value (while pinned, value can't be freed)
6. Unpin epoch
```
No lock taken. No CAS. Pure atomic loads. Wait-free for readers.

**SET (Update Existing Key) — Lock-Free Swap:**
```
1. hash(key) → select shard → linear probe → find Entry
2. Allocate new ValueBox (from thread-local pool if available)
3. entry.value.swap(new_ptr, AcqRel) → get old_ptr
4. If old_ptr != null:
     retire old_ptr into thread-local garbage list
5. Return
```
Single atomic swap. No CAS loop needed for updates. The old value is safe to free after all readers unpin.

**SET (New Key) — CAS Insert:**
```
1. hash(key) → select shard → linear probe → all slots occupied or null found
2. Reserve capacity: CAS on shard.len (increment by 1)
     if len >= threshold → return Err(Full)
3. Allocate Entry on heap (hash + key + AtomicPtr to ValueBox)
4. CAS slot from null → Entry*
     if CAS fails (another thread took this slot) → retry next slot
     if CAS succeeds → done, key is visible to all readers
```
The slot transitions null → Entry* exactly once. This is the key invariant that makes the map lock-free: slots never go back to null, entries never move.

**REMOVE — Atomic Null:**
```
1. Find entry via linear probe
2. entry.value.swap(null, AcqRel) → get old_ptr
3. Decrement key_count
4. Retire old_ptr via EBR
```
The Entry stays in its slot forever (its key remains for probing). Only the value pointer is nulled. This means `contains_key` returning false doesn't free the slot — this is the trade-off for lock-freedom without tombstone complexity.

### Lock-Free Invariants

1. **Slots are write-once per table** — within a single SlotTable, a slot transitions `null → Entry*` exactly once via CAS, never back. No ABA problem.
2. **Keys are immutable** — once an Entry is published, its `key` field never changes. Readers can compare keys without synchronization.
3. **Values swap atomically** — single `swap` or `compare_exchange` on `AtomicPtr<ValueBox<V>>`.
4. **Table pointer swaps atomically** — on grow, the shard's `AtomicPtr<SlotTable>` is swapped to point to a larger table. Readers on the old table still see valid Entry pointers (entries are shared, not copied).
5. **No per-shard lock on normal ops** — readers and writers operate simultaneously without coordination. The grow_lock is only taken during resize (rare, O(log N) times total).

### Dynamic Growth (Like Redis)

```
Initial state:
  Shard.table → SlotTable (capacity = max_keys / num_shards)

When shard reaches 70% occupancy:
  1. grow_lock.lock()           — only one thread grows at a time
  2. If threshold already raised (another thread grew first) → return
  3. Allocate new SlotTable at 2× capacity
  4. Copy all live Entry pointers from old table to new table
     (Entry objects are shared — same heap allocation, just referenced from new position)
  5. shard.table.store(new_ptr, Release)   — readers instantly see new table
  6. Old SlotTable (just the pointer array) is leaked
     (safe: readers may still be probing it; entries are alive in new table)
  7. grow_lock.unlock()

After grow:
  - Readers already on old table: still see valid Entry pointers (shared)
  - New readers: use new table (larger, more room)
  - Writers retrying: see new threshold, insert into new table

Memory lifecycle:
  - SlotTable arrays: leaked on grow (8 bytes × old_capacity, ~few KB each)
  - Entry objects: live forever once inserted (key stays for probing)
  - ValueBox: swapped atomically, recycled via EBR pool
  - String data inside values: freed when ValueBox is reclaimed
```

**On restart with RDB**: if `FLASHDB_MAX_KEYS` is configured to match your data size, the table starts pre-sized and no grows happen during load. If keys exceed the configured max, the table grows automatically — no crash, no data loss.

**On DELETE**: the Entry struct stays in its slot (key needed for linear probing). The ValueBox is freed via EBR. The slot array never shrinks — same as Redis. Memory is fully reclaimed only on restart.

### Epoch-Based Reclamation (EBR)

When a value is swapped out, the old `ValueBox` can't be freed immediately — another thread might still be reading it. EBR solves this:

```
Global State:
  GLOBAL_EPOCH: AtomicU64          (starts at 1, only advances forward)
  PARTICIPANTS: lock-free linked list of per-thread Participant nodes

Per-Thread State (thread_local!):
  participant: *Participant         (node in global list, holds local epoch)
  garbage: Vec<Garbage>             (retired pointers waiting to be freed)
  pool: Vec<(*mut ValueBox, ...)>   (recycled allocations, up to 1024)
  depth: usize                      (pin nesting counter)
  retires: usize                    (collection trigger counter)
```

**Pin/Unpin Protocol:**
```
pin():
  depth += 1
  if depth == 1:
    loop:
      e = GLOBAL_EPOCH.load(Relaxed)
      participant.local.store(e, Release)
      fence(Acquire)
      if GLOBAL_EPOCH.load(Acquire) == e: break
      participant.local.store(INACTIVE, Release)

unpin():
  depth -= 1
  if depth == 0:
    participant.local.store(INACTIVE, Release)
```

**Retirement:**
```
retire(ptr):
  push Garbage { ptr, epoch: GLOBAL_EPOCH.load(), drop_fn } to thread-local list
  retires += 1
  if retires % 512 == 0: collect()
```

**Collection (amortized):**
```
collect():
  1. Adopt orphan garbage from terminated threads
  2. Check if all participants have caught up to global epoch
     (all local epochs == INACTIVE or >= global)
  3. If yes: advance global epoch by 1
  4. Free all garbage where (garbage.epoch + 2 <= current_epoch)
     → but instead of freeing, push to pool (up to 1024 entries)
     → if pool full, actually drop the allocation
```

**Grace Period Guarantee:** A pointer retired at epoch E is safe to free at epoch E+2, because:
- At E: the retiring thread sees epoch E
- At E+1: all threads that were pinned at E have since unpinned (they saw E)
- At E+2: no thread can hold a reference from epoch E

**Value Pooling (Zero-Malloc Updates):**
```
replace_value(slot, new_value):
  1. Check pool for a ValueBox with matching TypeId
  2. If found: drop old content in-place, write new value → reuse allocation
  3. If not found: Box::new(ValueBox(value))
  4. swap(slot, new_ptr)
  5. retire(old_ptr) → old_ptr goes to garbage, eventually back to pool
```
Steady-state SET operations allocate zero memory — they recycle ValueBoxes from the pool.

### Performance Characteristics

| Operation               | Mechanism                                 | Cost   |
| ----------------------- | ----------------------------------------- | ------ |
| Read (find entry)       | Wait-free linear probe, Acquire loads     | ~30ns  |
| Read (get value clone)  | Pin + load + clone + unpin (one TLS)      | ~50ns  |
| Write (update existing) | Alloc from pool + swap + retire (one TLS) | ~60ns  |
| Write (new key)         | CAS on len + CAS on slot + heap alloc     | ~120ns |
| Remove                  | Swap to null + retire                     | ~50ns  |
| Contains                | Wait-free probe, no pin needed            | ~25ns  |

### Why Not DashMap / RwLock / Mutex

| Aspect                | DashMap (sharded RwLock)   | CustomMap (lock-free)                   |
| --------------------- | -------------------------- | --------------------------------------- |
| Read contention       | RwLock per shard (shared)  | No lock at all — pure atomic loads      |
| Write contention      | Exclusive lock per shard   | CAS on single slot / single swap        |
| Multiple readers      | All blocked during write   | Readers never wait for anything         |
| Memory reclamation    | Immediate (under lock)     | Deferred (EBR) — amortized to zero      |
| Value access          | Returns guard (holds lock) | Returns ref (holds lightweight pin)     |
| False sharing         | Shard structs may share    | 128-byte aligned, CachePadded atomics   |
| Throughput (12 cores) | ~7.5M SET/s                | ~15.9M SET/s                            |
| Internal (no TCP)     | ~20M SET/s                 | ~30M SET/s                              |

---

## Pub/Sub — Lock-Free Snapshot Architecture

The pub/sub system delivers messages from publishers to subscribers without any lock on the publish hot path.

### Design

```
PubSub
  ├── shards: [ChannelShard; 64]     (channel name → shard via foldhash)
  ├── patterns: RwLock<Vec<Pattern>>  (pattern subscriptions, rare)
  └── hasher: foldhash::RandomState

ChannelShard
  ├── snapshot: AtomicPtr<Vec<ChannelData>>  (read by publish, lock-free)
  └── mu: Mutex<Vec<*mut Vec<ChannelData>>>  (write by subscribe, holds retired snapshots)

ChannelData
  ├── name: String
  └── slots: Vec<Arc<SubSlot>>

SubSlot (per-subscriber)
  ├── queue: SegQueue<Arc<[u8]>>   (lock-free MPMC queue)
  ├── token: usize                  (connection ID for waker)
  ├── notify_pending: AtomicBool    (coalesced wake flag)
  └── notifier: Arc<WorkerNotifier> (epoll waker)
```

### Publish Path (Zero Locks)

```
PUBLISH channel message:
  1. hash(channel) → select ChannelShard
  2. ptr = shard.snapshot.load(Acquire)           // single atomic load
  3. Iterate Vec<ChannelData> at *ptr:
       if entry.name == channel:
         frame = Arc::new(encode_message(channel, message))  // allocate once
         for each SubSlot in entry.slots:
           slot.queue.push(Arc::clone(frame))    // lock-free push
           if !slot.notify_pending.swap(true):
             waker.wake()                        // one syscall per drain cycle
         return subscriber_count
  4. Check patterns (RwLock read, only if patterns exist)
```

The publish path touches:
- One `AtomicPtr::load` (no cache contention — ptr changes only on subscribe/unsubscribe)
- One heap allocation for the frame
- N `Arc::clone` + N `SegQueue::push` (where N = subscribers)
- At most one `wake()` syscall per subscriber per batch

**No Mutex. No RwLock. No CAS loop.** The snapshot pointer is stable during publish — it only changes when subscribe/unsubscribe runs.

### Subscribe/Unsubscribe (Copy-on-Write)

```
SUBSCRIBE channel:
  1. Lock shard.mu (rare operation, only on sub/unsub)
  2. Read current snapshot
  3. Build new Vec<ChannelData> with the new SubSlot appended
  4. shard.snapshot.store(new_ptr, Release)   // publish instantly sees new list
  5. Push old_ptr to retired list (freed after 4 generations)
  6. Unlock
```

Old snapshots are kept alive in a retire list (max 4) because a concurrent `publish()` might still be iterating the old snapshot. After 4 new subscribe/unsubscribe operations on the same shard, the oldest snapshot is freed. This is a simplified form of RCU (read-copy-update).

### Message Delivery (Worker Side)

```
Worker event loop:
  1. epoll wakes on WAKER_TOKEN (subscriber has pending messages)
  2. For each dirty connection:
       conn.do_write():
         if wbuf < 512KB:                    // backpressure: don't drain if buffer full
           slot.drain_into(wbuf)             // move all queued frames to write buffer
         write(socket, wbuf)                 // flush to TCP
  3. If any subscriber has pending data:
       poll with 100µs timeout (don't block forever)
       actively retry writes on all pending connections
```

The 512KB backpressure limit prevents unbounded memory growth when a subscriber reads slowly. The 100µs timeout ensures the server keeps pushing data even when epoll doesn't fire (subscriber socket buffer drains from the client side, making space available).

---

## Source Layout

```
src/
├── main.rs                   Entry point: config, signal handling, thread spawn
├── lib.rs                    Module declarations
├── worker.rs                 Per-thread epoll loop, write batching, subscriber flush
│
├── handler/
│   ├── conn.rs               Conn struct, do_read/do_write, inline SET/GET fast path
│   ├── dispatch.rs           Command routing (byte comparison, no allocation)
│   ├── subscription.rs       SUBSCRIBE/UNSUBSCRIBE state machine
│   └── pubsub_cmds.rs        PUBSUB CHANNELS/NUMSUB/NUMPAT
│
├── commends/
│   ├── mod.rs                ComdType enum + execute() dispatcher
│   ├── connection.rs         PING, ECHO, INFO, FLUSH, DBSIZE, TYPE, BGSAVE
│   ├── string.rs             SET (with NX/XX/GET/EX/PX), GET, MSET, INCR, etc.
│   ├── keys.rs               DEL, EXPIRE, RENAME, COPY, RANDOMKEY, KEYS
│   ├── hash.rs               HSET, HGET, HGETALL, HINCRBY, etc.
│   └── scan.rs               SCAN with shard/slot cursor
│
├── storage/
│   ├── store.rs              Store struct — Arc<CustomMap<StoreValue>>
│   ├── value.rs              FlashDB enum + StoreValue + TTL helpers
│   ├── string.rs             Atomic string ops (getset via CAS, setnx, try_set_string)
│   ├── hash.rs               Hash ops (clone-and-swap via try_update)
│   ├── keys.rs               Key ops (random via reservoir sampling, exists via precise TTL)
│   ├── scan.rs               Hash-based cursor scan (shard<<20 | slot)
│   ├── server.rs             INFO, FLUSH, cleanup_expired
│   └── rdb.rs                RDB save (fsync + dir sync), load (bounded, hardened)
│
├── pubsub/
│   ├── registry.rs           Lock-free snapshot pub/sub (AtomicPtr + copy-on-write)
│   ├── slot.rs               SubSlot (SegQueue + AtomicBool notify + queue_len)
│   └── frame.rs              RESP message encoding
│
├── macros/
│   ├── cmd.rs                parse_int!, parse_float!, wt!, store_ok!
│   ├── string_enum.rs        Zero-alloc command enum from bytes
│   └── sub.rs                Pub/sub reply macros
│
└── utils/
    ├── parser.rs             Zero-copy RESP parser (SIMD memchr for \n scan)
    ├── resp.rs               Raw byte response builders (cached integers, bulk, etc.)
    └── util.rs               glob_match, format_float

crates/customhash/src/
├── lib.rs                    CustomMap<V> — lock-free sharded open-addressing hash map
└── ebr.rs                    Epoch-based reclamation with thread-local value pooling
```

---

## Request Lifecycle

```
Client (redis-cli / any RESP client)
    │
    │  TCP  (TCP_NODELAY, SO_REUSEPORT)
    ▼
Per-thread TcpListener                         [worker.rs]
    │  Kernel distributes connections via SO_REUSEPORT
    ▼
mio epoll event loop                           [worker.rs]
    │  Read pass: process all ready events in batch
    │  Write pass: flush all dirty connections
    │  Subscriber pass: retry pending writes (100µs poll)
    ▼
Conn::do_read()                                [handler/conn.rs]
    │  read(socket) → parser buffer
    │  parse_one() loop: extract commands from RESP stream
    ▼
Inline fast path (SET/GET)                     [handler/conn.rs]
    │  Check first 3 bytes directly from raw pointer
    │  SET: store.set_string(key, value, 0) → append "+OK\r\n"
    │  GET: store.get_to_buf(key, wbuf) → zero-copy bulk write
    │  No &str array, no enum, no allocation
    ▼
General dispatch (other commands)              [handler/dispatch.rs → commends/]
    │  Build &str array on stack (32-element fixed array)
    │  cmd_eq() byte comparison → route to handler
    ▼
Store operation                                [storage/ → customhash/]
    │  hash(key) → shard → linear probe → atomic operation
    │  EBR pin/unpin around value access
    ▼
Conn::do_write()                               [handler/conn.rs]
    │  write(socket, wbuf) in loop until WouldBlock or empty
    ▼
Client receives response
```

---

## Concurrency Model

```
main thread
    ├── N worker threads: epoll loop (one per CPU core, SO_REUSEPORT)
    ├── expiry thread: cleanup_expired() every 1s via retain()
    ├── RDB saver thread: save() every 300s (fsync + atomic rename)
    └── signal thread: SIGTERM/SIGINT → save → exit

All workers share (read-mostly, lock-free):
    ├── Arc<Store> → Arc<CustomMap<StoreValue>>
    └── Arc<PubSub> → lock-free snapshot channel registry
```

No global lock on any hot path. Two clients writing different keys → zero contention. Two clients writing the same key → contend on one atomic swap (not a lock, just a retry).

---

## Persistence — RDB Snapshots

Format: `FLDB` magic (4 bytes) + version (1 byte) + entries + `0xFF` EOF marker.

Each entry: `type(1) + ttl_ms(8) + key_len(4) + key_bytes + value_payload`

Safety features:
- **Atomic write**: write to `.tmp` → fsync file → fsync directory → rename. Crash mid-save never corrupts existing snapshot.
- **Bounded load**: string lengths capped at 512MB, hash field counts at 10M. Rejects corrupted files.
- **Truncation recovery**: if EOF marker is missing, loads what's available and logs warning.
- **Expired skip**: keys past TTL at load time are skipped without allocation.

Triggers: startup load, periodic (configurable), BGSAVE command, SIGTERM/SIGINT.

---

## Complexity Reference

| Operation          | Time     | Notes                                      |
| ------------------ | -------- | ------------------------------------------ |
| GET / EXISTS / TTL | O(1) avg | Lock-free probe + atomic load              |
| SET / DEL / EXPIRE | O(1) avg | Lock-free probe + atomic swap              |
| INCR / APPEND      | O(1) avg | CAS loop via try_update                    |
| HGET / HSET / HDEL | O(1) avg | Outer probe + inner HashMap clone-and-swap |
| HGETALL            | O(f)     | f = number of fields                       |
| MGET / MSET        | O(k)     | k = number of keys                         |
| KEYS pattern       | O(n)     | Full scan + glob match                     |
| SCAN cursor        | O(count) | Hash-based cursor, stable across mutations |
| PUBLISH            | O(s)     | s = subscribers (lock-free snapshot read)  |
| RANDOMKEY          | O(n)     | Reservoir sampling in single pass          |
| cleanup_expired    | O(n)     | retain() scan, runs off hot path (1s)      |
| RDB save/load      | O(n)     | Sequential scan + 1MB buffered I/O + fsync |
| RESP parse         | O(b/32)  | b = bytes, SIMD memchr for \n              |
