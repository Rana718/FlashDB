# FlashDB — Architecture

## Overview

FlashDB is a Redis-compatible in-memory key-value store written in Rust. It speaks the RESP wire protocol so any Redis client works out of the box. It uses a thread-per-core event loop architecture, a fully lock-free concurrent hash map with epoch-based reclamation, zero-copy RESP parsing, and direct-write response building.

---

## Benchmark Results

Peak observed on a warmed 6-core Intel i5-11400H (12 hardware threads) using
loopback TCP, 100 clients, and three complete benchmark runs. Figures are
workload-specific measurements, not latency or throughput guarantees.

| Metric           | FlashDB (6 cores) | Redis Cluster (6 nodes) | vs Cluster |
| ---------------- | ----------------- | ----------------------- | ---------- |
| Pipeline-64 SET  | ~14.7M ops/sec    | ~3.5M ops/sec           | 4.2x       |
| Pipeline-100 SET | ~14.9M ops/sec    | ~7.9M ops/sec           | 1.9x       |
| Pipeline-100 GET | ~19.3M ops/sec    | ~8.3M ops/sec           | 2.3x       |
| Pub/Sub delivery | ~36.8M msg/sec    | ~7.3M msg/sec           | 5.0x       |

### Resource Usage

| Measurement             | Result  |
| ----------------------- | ------- |
| Idle RSS (no keys)      | ~55 MB          |
| Average RSS under load  | ~215 MB         |
| Peak RSS during a run   | ~235 MB         |
| Average CPU under load  | ~50%            |
| Peak CPU during a run   | ~60%            |

---

## Why It's Fast

| Factor            | Redis                    | FlashDB                                               |
| ----------------- | ------------------------ | ----------------------------------------------------- |
| I/O model         | Single-thread epoll      | Thread-per-core epoll (mio), SO_REUSEPORT             |
| Accept queue      | Single shared queue      | Per-thread kernel queue — no contention               |
| Hash map          | Custom, single-threaded  | Lock-free CustomMap — EBR, atomic swap, value pooling |
| RESP parsing      | Copy into dynamic buffer | Zero-copy: parse directly from read buffer            |
| Command dispatch  | String comparison        | First-byte fast-path for top-7 commands, length-gated enum fallback |
| Response building | Format into String       | Inline bulk headers for lengths < 100, raw byte writes |
| GET path          | Clone value + allocate   | Zero-copy: write directly from stored value to buffer |
| MGET path         | N clones + array alloc   | N direct-to-buffer writes, zero intermediate allocation |
| Mutations         | Copy entire value        | In-place mutation via update_with + CAS (INCR, HSET, EXPIRE, APPEND) |
| TTL computation   | clock_gettime per op     | Cached clock ticked once per event loop iteration     |
| Allocator         | libc malloc              | mimalloc — faster small allocation                    |
| Memory search     | Naive byte scan          | SIMD memchr (AVX2) for newline scanning               |
| Write batching    | Per-command write        | Batched: all events read first, then flush all writes |
| Write buffer      | Carry offset             | Drain written bytes on partial write, zero dead bytes |
| Pub/Sub fan-out   | Per-message frame copy   | Single Arc<[u8]> allocation, shared to all subscribers |
| Pub/Sub dispatch  | Single-thread loop       | Lock-free Arc snapshot, first-byte pattern index      |
| Pub/Sub notify    | Wake per message         | Coalesced notifications, deduped dirty list           |
| Memory limit      | maxmemory config         | max_keys enforcement via try_insert/try_set           |

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
  ├── key_count: AtomicUsize          (global live key count)
  └── max_keys: usize                 (capacity limit, enforced on try_insert/try_set)

Shard<V>  (128-byte aligned, eliminates false sharing between shards)
  ├── table: AtomicPtr<SlotTable<V>>       (swapped on growth)
  ├── len: CachePadded<AtomicUsize>        (occupied slots including tombstones)
  ├── insert_gate: CachePadded<AtomicUsize> (concurrent insert counter + GROWING flag)
  └── grow_lock: Mutex<()>                  (serializes growth, never held during ops)

SlotTable<V>
  ├── slots: Box<[AtomicPtr<Entry<V>>]>   (fixed-size array, open addressing)
  ├── mask: usize                          (capacity - 1, for fast modulo)
  └── threshold: usize                     (capacity × 70% = max occupancy)

Entry<V>  (heap-allocated, pointed to by slot)
  ├── hash: u64                      (full 64-bit hash, cached for fast compare)
  ├── key: String                    (immutable after insertion, never changes)
  └── value: AtomicPtr<ValueBox<V>>  (the only mutable field — swapped atomically)
```

### How Operations Work

**Shard Selection (O(1)):**
```
hash = foldhash(key)
shard_idx = (hash >> shift) & shard_mask
```
Upper bits select the shard (better distribution), lower bits determine probe position within the shard.

**GET — Lock-Free Read:**
```
1. hash(key) → select shard
2. Linear probe from (hash & mask):
     load AtomicPtr<Entry> at slot[i]
     if null → key doesn't exist, return None
     if entry.hash == hash && entry.key == key → found
     else i = (i + 1) & mask
3. Pin EBR epoch (single TLS access)
4. Load entry.value (AtomicPtr, Acquire ordering)
5. Read/clone the value (while pinned, value can't be freed)
6. Unpin epoch
```
No lock. No CAS. Pure atomic loads. Wait-free for readers.

**SET (Update Existing Key) — Lock-Free Swap:**
```
1. hash(key) → select shard → linear probe → find Entry
2. Allocate new ValueBox (from thread-local pool if available)
3. entry.value.swap(new_ptr, AcqRel) → get old_ptr
4. If old_ptr != null: retire old_ptr into thread-local garbage list
5. Return
```
Single atomic swap. The old value is safe to free after all readers unpin.

**SET (New Key) — CAS Insert:**
```
1. hash(key) → select shard → linear probe → find null slot
2. enter_insert() — increment insert_gate (blocks growth during insert)
3. Reserve capacity: CAS on shard.len (increment by 1)
     if len >= threshold → drop guard, trigger grow, retry
4. Allocate Entry on heap (hash + key + AtomicPtr to ValueBox)
5. CAS slot from null → Entry*
     if CAS fails → retry next slot
     if CAS succeeds → done
6. drop InsertGuard (decrement insert_gate)
```

**REMOVE — Atomic Null:**
```
1. Find entry via linear probe
2. entry.value.swap(null, AcqRel) → get old_ptr
3. Decrement key_count
4. Retire old_ptr via EBR
```
The Entry stays in its slot (key remains for probing). Only the value pointer is nulled.

**UPDATE_WITH — In-Place Mutation:**
```
1. Find entry via linear probe
2. Load current value pointer
3. Clone value, apply mutation function on the clone
4. CAS old_ptr → new_ptr
     if CAS fails (concurrent modification) → retry from step 2
5. Retire old_ptr
```
Used by INCR, HINCRBY, HDEL, HSET — avoids caller-side cloning.

### Lock-Free Invariants

1. **Slots are write-once per table** — a slot transitions `null → Entry*` exactly once via CAS, never back. No ABA problem.
2. **Keys are immutable** — once an Entry is published, its `key` field never changes.
3. **Values swap atomically** — single `swap` or `compare_exchange` on `AtomicPtr<ValueBox<V>>`.
4. **Table pointer swaps atomically** — on grow, the shard's `AtomicPtr<SlotTable>` is swapped. Readers on the old table still see valid Entry pointers (shared).
5. **No per-shard lock on normal ops** — readers and writers operate simultaneously. The grow_lock is only taken during resize.

### Dynamic Growth

```
When shard reaches 70% occupancy:
  1. grow_lock.lock()
  2. Set GROWING bit in insert_gate (spin-waits all concurrent inserters)
  3. Wait for all active inserters to complete (gate reaches GROWING | 0)
  4. Allocate new SlotTable at 2× capacity
  5. Copy live Entry pointers (skip tombstones — value == null)
  6. Recount live entries → store as new shard.len
  7. shard.table.store(new_ptr, Release)
  8. Retire old SlotTable through EBR
  9. Clear GROWING bit (inserters resume)
  10. grow_lock.unlock()
```

Tombstone compaction happens during growth: entries with null values are not copied, and `shard.len` is corrected to reflect only live entries. This prevents premature growth under key churn.

### Capacity Enforcement

`try_insert` and `try_set` check `key_count >= max_keys` before inserting new keys. Updates to existing keys always succeed (they don't increase count). When at capacity, new keys receive `-OOM command not allowed: store is at capacity`.

### Epoch-Based Reclamation (EBR)

When a value is swapped out, the old `ValueBox` can't be freed immediately — another thread might still be reading it.

```
Global State:
  GLOBAL_EPOCH: AtomicU64          (monotonically increasing)
  PARTICIPANTS: lock-free linked list of per-thread nodes

Per-Thread State (thread_local!):
  participant.local: AtomicU64     (current epoch or INACTIVE)
  garbage: Vec<Garbage>            (retired pointers with epoch stamps)
  pool: Vec<ValueBox>              (recycled allocations, up to 1024)
  depth: usize                     (pin nesting counter)
```

**Pin/Unpin:**
```
pin():
  depth += 1
  if depth == 1:
    load global epoch → store into participant.local (Release)
    fence(Acquire) + verify epoch didn't advance

unpin():
  depth -= 1
  if depth == 0:
    participant.local.store(INACTIVE, Release)
```

**Collection (every 512 retires):**
```
1. Adopt orphan garbage from terminated threads
2. Check if all participants are at or past current epoch
3. If yes: advance global epoch
4. Free garbage where (epoch + 2 <= current_epoch)
   → Recycle into pool if TypeId matches and pool < 1024
   → Otherwise drop
```

**Grace Period:** A pointer retired at epoch E is safe to free at epoch E+2.

**Value Pooling:** Steady-state SET operations allocate zero memory — ValueBoxes are recycled from the thread-local pool.

### Performance Characteristics

| Operation               | Mechanism                                 | Cost   |
| ----------------------- | ----------------------------------------- | ------ |
| Read (find entry)       | Wait-free linear probe, Acquire loads     | ~30ns  |
| Read (get value clone)  | Pin + load + clone + unpin                | ~50ns  |
| Write (update existing) | Alloc from pool + swap + retire           | ~60ns  |
| Write (new key)         | CAS on len + CAS on slot + heap alloc     | ~120ns |
| Remove                  | Swap to null + retire                     | ~50ns  |
| Contains                | Wait-free probe, no pin needed            | ~25ns  |

---

## Pub/Sub — Lock-Free Arc Snapshot Architecture

The pub/sub system delivers messages from publishers to subscribers without any lock on the publish hot path.

### Design

```
PubSub
  ├── shards: [ChannelShard; 64]     (channel name → shard via foldhash)
  ├── patterns: RwLock<PatternIndex>  (first-byte indexed pattern buckets)
  └── hasher: foldhash::RandomState

ChannelShard
  ├── snapshot: AtomicPtr<Box<Arc<Vec<ChannelData>>>>  (read by publish via Arc clone)
  └── mu: Mutex<()>                                     (serializes subscribe/unsubscribe)

ChannelData
  ├── name: String
  └── slots: Vec<Arc<SubSlot>>

PatternIndex
  ├── buckets: [Vec<PatternEntry>; 256]  (indexed by first byte of pattern)
  ├── wildcard: Vec<PatternEntry>         (patterns starting with * ? [)
  └── total: usize

SubSlot (per-subscriber)
  ├── queue: SegQueue<Arc<[u8]>>   (lock-free MPMC queue)
  ├── token: usize                  (connection ID for waker)
  ├── notify_pending: AtomicBool    (coalesced wake flag)
  ├── len: AtomicUsize              (queue length for backpressure)
  └── notifier: Arc<WorkerNotifier> (epoll waker)
```

### Publish Path (Zero Locks)

```
PUBLISH channel message:
  1. hash(channel) → select ChannelShard
  2. Arc clone of current snapshot (single atomic increment)
  3. Iterate Vec<ChannelData>:
       if entry.name == channel:
         frame = encode_message(channel, message) → Arc<[u8]>
         for each SubSlot in entry.slots:
           slot.queue.push(Arc::clone(frame))
           if !slot.notify_pending.swap(true): waker.wake()
         return subscriber_count
  4. Check patterns (RwLock read, only if patterns exist):
       - Check wildcard bucket (patterns starting with * ? [)
       - Check bucket[channel_first_byte] (patterns sharing first char)
       - Skip all other buckets
```

No Mutex. No CAS loop. The Arc snapshot ensures the channel list stays alive for the entire duration of the publish, even if concurrent subscribe/unsubscribe replaces it.

### Subscribe/Unsubscribe (Copy-on-Write)

```
SUBSCRIBE channel:
  1. Lock shard.mu
  2. Arc::clone current snapshot → build new Vec with slot appended
  3. Create new Arc<Vec<ChannelData>>
  4. Swap AtomicPtr to new Box<Arc<...>>
  5. Drop old Box (the Arc inside may still be alive if a publisher holds it)
  6. Unlock
```

Publishers holding an Arc clone of the old snapshot keep it alive until they finish iterating. No use-after-free, no heuristic retirement limits.

### Message Delivery

```
Worker event loop:
  1. epoll wakes on WAKER_TOKEN (subscriber has pending messages)
  2. Dedup dirty subscriber list (sort + dedup, no duplicate processing)
  3. For each dirty connection:
       conn.do_write():
         if wbuf < 256KB: slot.drain_into_limit(wbuf, 256KB)
         write(socket, wbuf) in loop
         on partial write: drain written bytes from front
  4. Slow subscriber detection:
       if slot.queue_len > 262144: disconnect (prevent unbounded memory)
  5. If pending data remains: poll with 50µs timeout for active retry
```

---

## Source Layout

```
src/
├── main.rs                   Entry point: config, signal handling, thread spawn
├── lib.rs                    Module declarations
├── worker.rs                 Per-thread epoll loop, clock tick, write batching
│
├── handler/
│   ├── conn.rs               Conn struct, do_read/do_write with byte drain, inline SET/GET
│   ├── dispatch.rs           Fast-path (SET/GET/DEL/INCR/HSET/HGET/EXPIRE) + enum fallback
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
│   ├── store.rs              Store struct (CustomMap + client counter + memory counter)
│   ├── value.rs              FlashDB enum + StoreValue + cached clock + TTL helpers
│   ├── string.rs             Atomic string ops (set, get_to_buf, in-place int_op/append/setrange)
│   ├── hash.rs               Hash ops (in-place mutation via update_with)
│   ├── keys.rs               Key ops (O(1) randomkey, in-place expire/persist, rename)
│   ├── scan.rs               Hash-based cursor scan (shard<<20 | slot)
│   ├── server.rs             INFO (O(1) via atomic counter), FLUSH, cleanup_expired
│   └── rdb.rs                RDB save (per-slot iteration, no global EBR hold), load
│
├── pubsub/
│   ├── registry.rs           Arc-snapshot pub/sub (AtomicPtr<Box<Arc<Vec>>>)
│   ├── slot.rs               SubSlot (SegQueue + notify coalescing + length tracking)
│   └── frame.rs              RESP message encoding (direct to Arc<[u8]>)
│
├── macros/
│   ├── cmd.rs                parse_int!, parse_float!, wt!, store_ok!
│   ├── string_enum.rs        Zero-alloc command enum from bytes
│   └── sub.rs                Pub/sub reply macros
│
└── utils/
    ├── parser.rs             Zero-copy RESP parser (SIMD memchr, auto-shrink)
    ├── resp.rs               Inline bulk headers, cached integers, raw byte builders
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
    │  tick_clock() — update cached timestamp (one store, no syscall)
    │  Read pass: process all ready events in batch
    │  Write pass: flush all dirty connections (drain written bytes)
    │  Subscriber pass: deduped dirty list, drain queues, retry pending
    ▼
Conn::do_read()                                [handler/conn.rs]
    │  read(socket) → parser buffer (auto-shrinks after large commands)
    │  parse_one() loop: extract commands from RESP stream
    ▼
Inline fast path (SET/GET 3-byte check)        [handler/conn.rs]
    │  SET (no options): set_string → "+OK\r\n" (zero alloc for existing keys)
    │  GET: get_to_buf → inline bulk header + direct value write
    ▼
Dispatch fast path (DEL/INCR/HSET/HGET/EXPIRE) [handler/dispatch.rs]
    │  First-byte match → direct handler call
    │  Skips ComdType enum parsing entirely
    ▼
General dispatch (remaining commands)          [commends/mod.rs]
    │  Length-gated enum match (only compares same-length commands)
    │  Stack-allocated &str array (32-element, no heap)
    ▼
Store operation                                [storage/ → customhash/]
    │  hash(key) → shard → linear probe → atomic operation
    │  update_with() for mutations (clone once inside CAS loop)
    │  EBR pin/unpin around value access
    ▼
Response encoding                              [utils/resp.rs]
    │  Inline bulk header for lengths < 100 (no digit reversal)
    │  Cached integer responses (:0 through :9)
    ▼
Conn::do_write()                               [handler/conn.rs]
    │  write(socket, wbuf) until WouldBlock or empty
    │  On partial write: drain written bytes from front
    │  Shrink wbuf if > 1MB after full drain
    ▼
Client receives response
```

---

## Concurrency Model

```
main thread
    ├── N worker threads: epoll loop (one per CPU core, SO_REUSEPORT)
    ├── expiry thread: cleanup_expired_shard() rotating every 100ms
    ├── RDB saver thread: save() every 300s (per-slot iteration, no global lock)
    └── signal thread: SIGTERM/SIGINT → initiate_shutdown → drain → save → exit

All workers share (lock-free):
    ├── Arc<Store> → CustomMap<StoreValue> + AtomicUsize counters
    └── Arc<PubSub> → Arc-snapshot channel registry

Shutdown sequence:
    1. Signal received (SIGTERM/SIGINT)
    2. Set global SHUTDOWN flag (AtomicBool)
    3. Workers detect flag at top of event loop
    4. Each worker flushes all pending write buffers
    5. Workers return (threads exit)
    6. Signal thread waits 100ms for drain
    7. Final RDB save
    8. Process exit
```

No global lock on any hot path. Two clients writing different keys → zero contention. Two clients writing the same key → contend on one atomic swap (not a lock, just a retry).

---

## Persistence — RDB Snapshots

Format: `FLDB` magic (4 bytes) + version (1 byte) + entries + `0xFF` EOF marker.

Each entry: `type(1) + ttl_ms(8) + key_len(4) + key_bytes + value_payload`

Safety:
- **Atomic write**: write to `.tmp` → fsync → rename. Crash mid-save never corrupts.
- **Per-slot iteration**: each `peek_slot()` call holds EBR briefly, allowing GC between slots. No multi-second GC stalls during save.
- **Bounded load**: string lengths capped at 512MB, hash field counts at 10M.
- **Truncation recovery**: if EOF marker is missing, loads what's available with warning.
- **Expired skip**: keys past TTL at load time are skipped without allocation.
- **BGSAVE dedup**: concurrent BGSAVE requests are rejected (AtomicBool guard).

Triggers: startup load, periodic (configurable), BGSAVE command, SIGTERM/SIGINT.

---

## Complexity Reference

| Operation          | Time     | Notes                                       |
| ------------------ | -------- | ------------------------------------------- |
| GET / EXISTS / TTL | O(1) avg | Lock-free probe + atomic load               |
| SET / DEL / EXPIRE | O(1) avg | Lock-free probe + atomic swap               |
| INCR / APPEND      | O(1) avg | In-place mutation via update_with + CAS     |
| HGET               | O(1) avg | Outer probe + inner HashMap lookup          |
| HSET / HDEL        | O(1) avg | In-place mutation via update_with + CAS     |
| HGETALL            | O(f)     | f = number of fields                        |
| MGET               | O(k)     | k direct-to-buffer writes, zero alloc       |
| MSET               | O(k)     | k = number of keys                          |
| MSETNX             | O(k)     | Atomic insert-and-rollback                  |
| KEYS pattern       | O(n)     | Per-shard scan + glob match (GC between)    |
| SCAN cursor        | O(count) | Hash-based cursor, stable across mutations  |
| PUBLISH            | O(s+p)   | s = subscribers, p = matching pattern bucket |
| RANDOMKEY          | O(1) avg | Random shard + sequential slot probe        |
| INFO               | O(1)     | Atomic memory counter read                  |
| cleanup_expired    | O(n/S)   | One shard per tick, rotating every 100ms    |
| RDB save/load      | O(n)     | Per-slot iteration + 1MB buffered I/O       |
| RESP parse         | O(b/32)  | b = bytes, SIMD memchr for \n               |
| Command dispatch   | O(1)     | Fast-path for top-7, length-gated enum rest |
