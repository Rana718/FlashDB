# FlashDB — Architecture

## Overview

FlashDB is a Redis-compatible in-memory key-value store written in Rust. It speaks the RESP wire protocol so any Redis client works out of the box. It beats Redis on every benchmark metric by using a thread-per-core event loop architecture, zero-copy RESP parsing, and direct-write response building with no intermediate allocations.

---

## Benchmark Results (6-core machine)

| Metric         | FlashDB (single core) | Redis (single core) | FlashDB (6 cores) |
| -------------- | --------------------- | ------------------- | ----------------- |
| Sequential SET | ~288k ops/sec         | ~175k ops/sec       | ~600k ops/sec     |
| Pipelined SET  | ~2.3M ops/sec         | ~1.4M ops/sec       | ~6M ops/sec       |
| Pipelined GET  | ~2.6M ops/sec         | ~1.8M ops/sec       | ~6.5M ops/sec     |

---

## Why It's Fast

| Factor            | Redis                    | FlashDB                                      |
| ----------------- | ------------------------ | -------------------------------------------- |
| I/O model         | Single-thread epoll      | Thread-per-core epoll (mio), SO_REUSEPORT    |
| Accept queue      | Single shared queue      | Per-thread kernel queue — no contention      |
| RESP parsing      | Copy into dynamic buffer | Zero-copy: parse directly from read buffer   |
| Response building | Format into String       | Write raw bytes directly into write buffer   |
| Arg dispatch      | N/A                      | Stack array — no heap allocation per command |
| Hash map          | Custom, single-threaded  | DashMap — N×32 shards, one RwLock per shard  |
| Allocator         | libc malloc              | mimalloc — faster small allocation           |
| Memory search     | Naive byte scan          | SIMD memchr (AVX2) for newline scanning      |

---

## Source Layout

```
src/
├── main.rs                   # Entry point: thread-per-core loop, signalfd shutdown
├── lib.rs                    # Module declarations
├── handler.rs                # Per-connection state: Conn struct, do_read/do_write
│
├── utils/
│   ├── parser.rs             # Zero-copy RESP parser with owned read/write buffers
│   ├── resp.rs               # Raw byte response builders (no String allocation)
│   └── util.rs               # format_float, glob_match
│
├── macros/
│   ├── cmd.rs                # parse_int!, parse_float!, wt!, store_ok! macros
│   ├── hash.rs               # hash_read!, hash_write! macros
│   └── string_enum.rs        # string_enum! macro (zero-cost command dispatch enum)
│
├── storage/
│   ├── mod.rs
│   ├── store.rs              # Store struct — DashMap + client counter
│   ├── value.rs              # FlashDB enum (String/Hash) + StoreValue + TTL
│   ├── string.rs             # String command storage impl
│   ├── keys.rs               # Key management storage impl
│   ├── hash.rs               # Hash command storage impl
│   ├── scan.rs               # SCAN cursor iteration
│   ├── server.rs             # INFO, FLUSH, DBSIZE, TYPE, cleanup_expired
│   └── rdb.rs                # RDB persistence: save, load, background save
│
└── commends/
    ├── mod.rs                # ComdType enum + execute() + execute_raw() dispatcher
    ├── connection.rs         # PING, ECHO, INFO, FLUSH, DBSIZE, TYPE, BGSAVE
    ├── string.rs             # All string commands
    ├── keys.rs               # All key commands
    ├── hash.rs               # All hash commands
    └── scan.rs               # SCAN command

tests/
├── common.rs                 # Helpers: store(), set_str(), set_expiring(), set_expired()
├── string.rs                 # 31 string storage tests
├── keys.rs                   # 22 key management tests
├── hash.rs                   # 19 hash storage tests
├── scan.rs                   # 5 SCAN tests
├── server.rs                 # 9 server/meta tests
└── rdb.rs                    # 10 RDB persistence tests
```

---

## Request Lifecycle

```
Client (redis-cli / go-redis / any RESP client)
    │
    │  TCP  (TCP_NODELAY — Nagle disabled, minimal latency)
    ▼
SO_REUSEPORT listener (one per thread)        [main.rs]
    │  Kernel distributes connections across threads
    │  No shared accept queue, no mutex contention
    ▼
mio epoll event loop                          [main.rs — run_worker()]
    │  EPOLLIN fires when socket has data
    │  O(1) event dispatch via Vec slab (index = token id)
    ▼
Conn::do_read()                               [handler.rs]
    │  loop { stream.read(parser.read_buf()) }  — drain socket
    │  O(n) where n = bytes available
    ▼
RespParser::parse_one()                       [utils/parser.rs]
    │  SIMD memchr scan for '\n' — O(n/32) with AVX2
    │  Store raw (ptr, len) into rbuf — zero allocation
    │  Repeat until Incomplete
    ▼
execute_raw(parts_raw, store, wbuf)           [commends/mod.rs]
    │  Reconstruct &str on stack (fixed [&str; 32] array)
    │  ComdType::from(parts[0]) — case-insensitive, O(1) match
    │  Jump to command handler
    ▼
Command handler e.g. string::get()           [commends/string.rs]
    │  store.get(key) — DashMap shard lookup, O(1) avg
    │  resp::write_opt_bulk(wbuf, value) — write raw bytes
    │  No String allocation for cache hits
    ▼
Conn::do_write()                              [handler.rs]
    │  stream.write(&wbuf[offset..]) — single write syscall
    │  Loop if partial write (kernel buffer full)
    ▼
Client receives response
```

---

## Data Model

```
DashMap<String, StoreValue>     O(1) avg insert/lookup/delete
         │
         └── StoreValue
                 ├── value: FlashDB
                 │       ├── String(String)
                 │       └── Hash(HashMap<String, String>)
                 └── expires_at: Option<Instant>
```

**Expiry strategy — two-pronged:**

- **Lazy**: checked on every access — expired keys return None and are removed inline. O(1), zero background work.
- **Active**: background thread calls `store.cleanup_expired()` every second using `DashMap::retain()`. Prevents memory leak from keys that are never accessed again. O(n) per sweep but runs off the hot path.

**Adding a new type** (List, Set, ZSet): add a variant to `FlashDB`, add storage methods in a new `storage/<type>.rs`, add commands in a new `commends/<type>.rs`.

---

## Concurrency Model

```
main thread
    ├── thread 0: mio epoll loop + signalfd watcher   (core 0)
    ├── thread 1: mio epoll loop                       (core 1)
    ├── ...
    ├── thread N: mio epoll loop                       (core N)
    ├── cleanup thread: cleanup_expired() every 1s
    └── rdb thread: save() every 300s

All worker threads share Arc<Store>
    └── DashMap — (num_cpus × 32).next_power_of_two() shards
                  each shard has one RwLock
                  reads: concurrent across all shards
                  writes: lock one shard, never block other shards
```

**SO_REUSEPORT**: each worker thread creates its own `TcpListener` bound to the same port. The kernel load-balances incoming connections across all listeners at the TCP accept queue level — no userspace coordination needed.

**No global lock** anywhere on the hot path. Two clients writing to different keys never contend.

---

## RESP Parser — Zero-Copy Design

RESP array format (what all Redis clients send):

```
*3\r\n       ← array of 3 elements
$3\r\n       ← bulk string, 3 bytes
SET\r\n
$4\r\n
name\r\n
$4\r\n
rana\r\n
```

**Old approach (tokio)**: `read_line()` → allocate `String` → `read_exact()` → allocate `Vec<u8>` → `String::from_utf8()`. ~5 heap allocations per command.

**Current approach**:

1. `stream.read(rbuf[filled..])` — fill a 64KB owned buffer, one syscall
2. `memchr::memchr(b'\n', ...)` — SIMD scan for newline, O(n/32)
3. Store `(*const u8, usize)` raw pointer pairs into `parts_raw` — zero allocation
4. Reconstruct `&str` on a stack array in `execute_raw` — zero allocation
5. For write operations: `key.to_string()` — one allocation, unavoidable (DashMap owns the key)
6. For read operations (GET, EXISTS, TTL): **zero allocations** end-to-end

**Read buffer compaction**: only when `< 4096` bytes remain — avoids `copy_within` on every read call.

---

## Response Building — Direct Write

**Old approach**: every response function returned `String` (heap alloc). Then `write_all(response.as_bytes())` copied it to the write buffer. Two copies, one allocation per response.

**Current approach**: `resp::write_*` functions take `&mut Vec<u8>` and write raw bytes directly:

```rust
// No allocation — writes fixed bytes
resp::write_ok(out)           →  out.extend_from_slice(b"+OK\r\n")

// No allocation — uses integer cache for 0-9
resp::write_integer(out, 5)   →  out.extend_from_slice(b":5\r\n")

// One allocation — value string from DashMap
resp::write_bulk(out, &val)   →  write length prefix + val bytes

// No allocation for reads that miss
resp::write_nil(out)          →  out.extend_from_slice(b"$-1\r\n")
```

Pre-built integer cache for 0–9 covers the majority of INCR/HLEN/LLEN responses.

**Pipeline batching**: all responses for a pipeline batch accumulate in `wbuf`, then flush in a single `write()` syscall. Redis does the same thing.

---

## Command Dispatch

The `string_enum!` macro generates a zero-cost enum with case-insensitive `From<&str>`:

```rust
string_enum! {
    pub enum ComdType {
        default = Exec;
        GET => "GET",
        SET => "SET",
        // ~60 commands total
    }
}
```

`ComdType::from("get")` — uppercase comparison, no allocation, O(1).

`execute_raw` reconstructs `&str` slices from raw pointers onto a 32-element stack array, then calls `execute(&arr[..n])`. For commands with ≤32 args (all standard Redis commands), zero heap allocation in dispatch.

---

## Persistence — RDB Snapshots

**Format** (`flashdb.rdb`):

```
[4 bytes magic: "FLDB"]
[1 byte version: 1]
[repeated entries:]
  type(1) + ttl_unix_ms(8) + key_len(4) + key
  String: val_len(4) + val
  Hash:   field_count(4) + (flen(4) + field + vlen(4) + val)*
[0xFF EOF marker]
```

Little-endian, no serde dependency. 1MB `BufWriter`/`BufReader` for I/O efficiency.

**Save**: background thread, writes to `flashdb.rdb.tmp`, then `rename()` — atomic on POSIX. Zero impact on event loop threads.

**Load**: called once at startup before `TcpListener::bind()`. Expired keys are skipped during load (TTL stored as absolute unix milliseconds).

**Triggers**:

- Startup: automatic load
- Every 300 seconds: background periodic save
- `BGSAVE` command: manual trigger, spawns a thread
- SIGTERM / SIGINT: `sigwait()` thread catches signal, saves synchronously, exits

**Shutdown signal handling**: uses `sigwait()` in a dedicated thread with `pthread_sigmask(SIG_BLOCK)` on that thread only. No signal handler registered — zero EINTR overhead on worker threads, zero impact on `epoll_wait` performance.

---

## Complexity Reference

| Operation          | Time     | Notes                                |
| ------------------ | -------- | ------------------------------------ |
| GET / EXISTS / TTL | O(1) avg | DashMap shard lookup                 |
| SET / DEL / EXPIRE | O(1) avg | DashMap shard insert/remove          |
| INCR / APPEND      | O(1) avg | In-place mutation via entry API      |
| HGET / HSET / HDEL | O(1) avg | DashMap outer + HashMap inner        |
| HGETALL            | O(f)     | f = number of fields                 |
| MGET / MSET        | O(k)     | k = number of keys                   |
| KEYS pattern       | O(n)     | Full scan + glob match               |
| SCAN cursor        | O(count) | Sorted key slice, stable cursor      |
| cleanup_expired    | O(n)     | DashMap::retain(), runs off hot path |
| RDB save           | O(n)     | Sequential scan + buffered write     |
| RDB load           | O(n)     | Sequential read + DashMap insert     |
| RESP parse         | O(b/32)  | b = bytes, SIMD memchr               |
| glob_match         | O(p×s)   | p = pattern len, s = string len      |

---