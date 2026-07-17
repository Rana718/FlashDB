# FlashDB — Architecture

## Overview

FlashDB is a Redis-compatible in-memory key-value store written in Rust. It accepts TCP connections, parses the RESP (Redis Serialization Protocol) wire format, executes commands against an in-memory store, and returns RESP responses. Any Redis client works without modification.

---

## Why It's Fast Compared to Redis

| Factor        | Redis                                   | FlashDB                                            |
| ------------- | --------------------------------------- | -------------------------------------------------- |
| Language      | C (single-threaded event loop)          | Rust (async multi-threaded)                        |
| Concurrency   | Single thread, I/O multiplexing         | One Tokio task per connection, fully parallel      |
| Memory safety | Manual, GC-free                         | Ownership system, zero GC pauses                   |
| Hash map      | Custom hash table with rehashing pauses | `DashMap` — lock-free sharded concurrent map       |
| I/O           | epoll event loop                        | Tokio async I/O with `BufReader`/`BufWriter`       |
| TCP           | Configurable                            | `TCP_NODELAY` always on — no Nagle buffering delay |
| Binary        | ~10MB with debug symbols                | `strip = true`, `opt-level = 3`, `lto = true`      |

Redis is single-threaded by design — one command runs at a time. FlashDB runs each client connection in its own Tokio task, so multiple commands execute concurrently. `DashMap` uses internal sharding to allow concurrent reads and writes without a global lock.

---

## Source Layout

```
src/
├── main.rs               # TCP listener, tokio runtime, expiry cleanup task
├── lib.rs                # module declarations
├── handler.rs            # per-connection loop (read → execute → write)
│
├── utils/
│   ├── parser.rs         # RESP protocol parser (buffered)
│   ├── resp.rs           # RESP response builder helpers
│   └── util.rs           # shared: format_float, glob_match
│
├── macros/
│   ├── string_enum.rs    # string_enum! macro (command name enum)
│   └── hash.rs           # hash_read! / hash_write! macros
│
├── storage/
│   ├── mod.rs            # module declarations
│   ├── store.rs          # Store struct (DashMap + client counter)
│   ├── value.rs          # FlashDB enum (String/Hash) + StoreValue
│   ├── string.rs         # string storage methods
│   ├── keys.rs           # key management storage methods
│   ├── hash.rs           # hash storage methods
│   ├── scan.rs           # SCAN cursor iteration
│   └── server.rs         # INFO, FLUSH, DBSIZE, TYPE, cleanup
│
└── commends/
    ├── mod.rs            # ComdType enum + execute() dispatcher
    ├── connection.rs     # PING, ECHO, INFO, FLUSH, DBSIZE, TYPE
    ├── string.rs         # all string commands
    ├── keys.rs           # all key commands
    ├── hash.rs           # all hash commands
    └── scan.rs           # SCAN command

tests/
├── common.rs             # shared test helpers (store(), set_str(), ...)
├── string.rs             # 31 string storage tests
├── keys.rs               # 22 key management tests
├── hash.rs               # 19 hash storage tests
├── scan.rs               # 5 SCAN tests
└── server.rs             # 9 server/meta tests
```

---

## Request Lifecycle

```
Client (redis-cli / go-redis / any RESP client)
    │
    │  TCP  (TCP_NODELAY = no buffering delay)
    ▼
TcpListener::accept()                        [main.rs]
    │
    └── tokio::spawn(handle_client)          [one task per connection]
            │
            ▼
        RespParser::parse()                  [utils/parser.rs]
            │  BufReader<OwnedReadHalf>
            │  reads *count\r\n then $len\r\ndata\r\n per token
            │  8KB read buffer — amortizes syscalls
            ▼
        Vec<String>  ["SET", "name", "rana"]
            │
            ▼
        commends::execute()                  [commends/mod.rs]
            │  ComdType::from("SET") → ComdType::SET
            │  match → string::set(parts, store)
            ▼
        store.set(key, StoreValue)           [storage/string.rs]
            │  DashMap::entry() — lock-free sharded insert
            ▼
        "+OK\r\n"
            │
            ▼
        RespParser::write_response()         [utils/parser.rs]
            │  BufWriter<OwnedWriteHalf>
            │  8KB write buffer — batches small writes
            ▼
Client receives response
```

---

## Data Model

```
DashMap<String, StoreValue>
           │
           └── StoreValue
                   ├── value: FlashDB        ← the actual data
                   │       ├── String(String)
                   │       └── Hash(HashMap<String, String>)
                   │
                   └── expires_at: Option<Instant>   ← TTL
```

**`FlashDB` enum** — adding a new type (List, Set, ZSet) means adding one variant here and implementing its methods in a new `storage/<type>.rs` file.

**Expiry** is lazy (checked on access) + active (background task sweeps every second).

---

## Concurrency Model

```
main thread
    └── tokio multi-thread runtime (N worker threads = CPU cores)
            ├── Task: expiry cleanup loop    (every 1 second)
            ├── Task: client A              (independent)
            ├── Task: client B              (independent)
            └── Task: client C              (independent)

All tasks share Arc<Store>
    └── DashMap — 64 shards, each with its own RwLock
                  reads on different shards are fully parallel
                  writes only lock one shard at a time
```

No global lock. Two clients writing to different keys never block each other.

---

## RESP Parser

FlashDB only handles RESP arrays — which is everything a Redis client sends.

```
*3\r\n          ← array of 3 elements
$3\r\n          ← bulk string, 3 bytes
SET\r\n
$4\r\n          ← bulk string, 4 bytes
name\r\n
$4\r\n
rana\r\n
```

The parser uses `tokio::io::BufReader` with an 8KB buffer. Instead of one syscall per byte (the old approach), it reads a full line at a time for headers and an exact chunk for data. This reduces syscalls by ~10x on typical commands.

---

## Command Dispatch

Commands are matched via the `string_enum!` macro which generates a zero-cost enum with `From<&str>` and case-insensitive matching:

```rust
string_enum! {
    pub enum ComdType {
        default = Exec;
        GET => "GET",
        SET => "SET",
        // ...
    }
}
```

`ComdType::from("get")` → `ComdType::GET` with no heap allocation. The match in `execute()` compiles to a jump table.

---

## Response Helpers

All RESP responses go through `utils/resp.rs` — a single source of truth:

```rust
resp::bulk("hello")       →  "$5\r\nhello\r\n"
resp::integer(42)         →  ":42\r\n"
resp::boolean(true)       →  ":1\r\n"
resp::opt_bulk(None)      →  "$-1\r\n"
resp::err("syntax error") →  "-ERR syntax error\r\n"
resp::wrong_type()        →  "-WRONGTYPE ...\r\n"
resp::wrong_args("get")   →  "-ERR wrong number of arguments for 'get' command\r\n"
```

---

## Storage Macros

Hash operations share a pattern: get entry → check expired → match type → execute. Two macros eliminate the boilerplate:

```rust
// Read from hash — returns Ok(default) if missing, Err if wrong type
hash_read!(self, key, default, |h| { /* use h: &HashMap */ })

// Write to hash — creates key if absent, Err if wrong type
hash_write!(self, key, |h| { /* use h: &mut HashMap */ })
```

This reduced ~120 lines of repeated match code across 8 methods to single-line calls.

---

## Benchmarking

```bash
cd bench
go run main.go  
```
