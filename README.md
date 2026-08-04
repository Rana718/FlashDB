# FlashDB

A Redis-compatible in-memory key-value store written in Rust. Speaks the RESP protocol so any Redis client works out of the box. Uses a fully lock-free concurrent hash map with epoch-based reclamation — no mutex, no RwLock on the data path.

## Performance

Benchmarked on a 12-core machine (Intel i5-11400H) with 100 clients, 1M ops, pipeline size 100.

| Metric           | FlashDB (6 cores) | Redis (1 core) | vs Redis |
| ---------------- | ------------------ | -------------- | -------- |
| Sequential SET   | ~860k ops/sec      | ~135k ops/sec  | 6.4x     |
| Pipelined SET    | ~9.7M ops/sec      | ~1.19M ops/sec | 8.2x     |
| Pipelined GET    | ~10.9M ops/sec     | ~1.68M ops/sec | 6.5x     |
| Pub/Sub delivery | ~12M msg/sec       | ~1.06M msg/sec | 11.3x    |

> Redis is single-threaded and does not scale with core count. FlashDB scales linearly with workers.

### Internal Store Throughput (no TCP overhead)

| Operation     | Throughput    |
| ------------- | ------------- |
| SET (new key) | 18.6M ops/sec |
| SET (update)  | 29.8M ops/sec |
| GET           | 34.3M ops/sec |

## Quick Start

```bash
cargo build --release
./target/release/flash_db

redis-cli -p 8000
127.0.0.1:8000> SET name rana
OK
127.0.0.1:8000> GET name
"rana"
127.0.0.1:8000> SUBSCRIBE news
127.0.0.1:8000> PUBLISH news "hello"
127.0.0.1:8000> BGSAVE
Background saving started
```

## Docker

```bash
docker run -p 8000:8000 rana718/flashdb:latest
```

## Key Design Decisions

- **Lock-free CustomMap** — custom concurrent hash map with epoch-based reclamation. No locks on read or write path. Values swapped atomically, old values freed after grace period.
- **Thread-per-core** — one epoll loop per CPU core, SO_REUSEPORT for kernel-level connection distribution.
- **Zero-copy GET** — reads write directly from stored value to TCP buffer. No String clone.
- **Inline SET/GET fast path** — hot commands dispatched from raw RESP bytes without building intermediate arrays.
- **Value pooling** — reclaimed allocations recycled in thread-local pool, eliminating malloc on update path.
- **Batched writes** — all epoll events processed before flushing responses, reducing syscall count.

## Persistence

FlashDB uses RDB snapshots — the same model as Redis.

- **On startup** — loads `flashdb.rdb` from the current directory if it exists
- **Every 5 minutes** — background save, zero impact on performance
- **On shutdown** — saves automatically on SIGTERM or Ctrl+C
- **Manual** — `BGSAVE` command triggers an immediate background save

The RDB file is written atomically (temp file → rename) so a crash mid-save never corrupts the existing snapshot.

## Supported Commands

### Connection

| Command      | Description                                  |
| ------------ | -------------------------------------------- |
| `PING [msg]` | Returns PONG or echoes msg                   |
| `ECHO msg`   | Returns msg as bulk string                   |
| `INFO`       | Server stats: version, memory, clients, keys |
| `DBSIZE`     | Number of keys in the store                  |
| `FLUSH`      | Delete all keys                              |
| `BGSAVE`     | Trigger a background RDB snapshot            |
| `TYPE key`   | Returns `string`, `hash`, or `none`          |

### String

| Command                                        | Description                           |
| ---------------------------------------------- | ------------------------------------- |
| `SET key value [EX s] [PX ms] [NX] [XX] [GET]` | Set a key with optional TTL and flags |
| `GET key`                                      | Get a value                           |
| `GETDEL key`                                   | Get and delete atomically             |
| `GETSET key value`                             | Get old value, set new value          |
| `GETEX key [EX s \| PX ms \| PERSIST]`         | Get value and update TTL              |
| `SETNX key value`                              | Set only if key does not exist        |
| `SETEX key seconds value`                      | Set key with TTL in seconds           |
| `PSETEX key ms value`                          | Set key with TTL in milliseconds      |
| `MSET key val [key val ...]`                   | Set multiple keys atomically          |
| `MSETNX key val [key val ...]`                 | Set multiple keys only if none exist  |
| `MGET key [key ...]`                           | Get multiple keys                     |
| `INCR key`                                     | Increment integer value by 1          |
| `DECR key`                                     | Decrement integer value by 1          |
| `INCRBY key n`                                 | Increment integer value by N          |
| `DECRBY key n`                                 | Decrement integer value by N          |
| `INCRBYFLOAT key n`                            | Increment float value by N            |
| `APPEND key value`                             | Append to string, returns new length  |
| `STRLEN key`                                   | String length in bytes                |
| `GETRANGE key start end`                       | Substring (supports negative indices) |
| `SETRANGE key offset value`                    | Overwrite bytes at offset             |

### Keys

| Command                             | Description                                        |
| ----------------------------------- | -------------------------------------------------- |
| `DEL key [key ...]`                 | Delete one or more keys, returns count deleted     |
| `UNLINK key [key ...]`              | Alias for DEL                                      |
| `EXISTS key [key ...]`              | Returns count of keys that exist                   |
| `TTL key`                           | TTL in seconds (-1 = no expiry, -2 = missing)      |
| `PTTL key`                          | TTL in milliseconds (-1 = no expiry, -2 = missing) |
| `EXPIRE key seconds`                | Set TTL in seconds                                 |
| `PEXPIRE key ms`                    | Set TTL in milliseconds                            |
| `EXPIREAT key unix`                 | Set expiry as Unix timestamp (seconds)             |
| `PERSIST key`                       | Remove TTL, make key persistent                    |
| `RENAME old new`                    | Rename key                                         |
| `RENAMENX old new`                  | Rename only if new key does not exist              |
| `COPY src dst [REPLACE]`            | Copy key to new key                                |
| `RANDOMKEY`                         | Return a random existing key                       |
| `KEYS pattern`                      | All keys matching glob pattern (`*`, `?`)          |
| `SCAN cursor [MATCH pat] [COUNT n]` | Cursor-based key iteration                         |

### Hash

| Command                                  | Description                                     |
| ---------------------------------------- | ----------------------------------------------- |
| `HSET key field value [field value ...]` | Set one or more fields, returns count added     |
| `HSETNX key field value`                 | Set field only if it does not exist             |
| `HGET key field`                         | Get field value                                 |
| `HMGET key field [field ...]`            | Get multiple fields                             |
| `HMSET key field value [...]`            | Set multiple fields (deprecated alias for HSET) |
| `HGETALL key`                            | Get all field/value pairs                       |
| `HDEL key field [field ...]`             | Delete fields, returns count deleted            |
| `HEXISTS key field`                      | Check if field exists                           |
| `HLEN key`                               | Number of fields                                |
| `HKEYS key`                              | All field names                                 |
| `HVALS key`                              | All field values                                |
| `HINCRBY key field n`                    | Increment integer field by N                    |
| `HINCRBYFLOAT key field n`               | Increment float field by N                      |

### Pub/Sub

| Command                            | Description                                       |
| ---------------------------------- | ------------------------------------------------- |
| `SUBSCRIBE channel [channel ...]`  | Subscribe to one or more channels                 |
| `UNSUBSCRIBE [channel ...]`        | Unsubscribe from channels (all if none specified) |
| `PSUBSCRIBE pattern [pattern ...]` | Subscribe to channels matching a glob pattern     |
| `PUNSUBSCRIBE [pattern ...]`       | Unsubscribe from patterns (all if none specified) |
| `PUBLISH channel message`          | Publish a message, returns number of receivers    |
| `PUBSUB CHANNELS [pattern]`        | List active channels with at least one subscriber |
| `PUBSUB NUMSUB [channel ...]`      | Subscriber count per channel                      |
| `PUBSUB NUMPAT`                    | Total number of pattern subscriptions             |

## Running Tests

```bash
cargo test
cargo test -- --quiet
cargo test rdb             # persistence tests only
cargo test pubsub          # pub/sub tests only
```

## Benchmarking

```bash
cd bench && go run .             # Full benchmark (KV + Pub/Sub)
cd bench && go run . -m key      # KV only
cd bench && go run . -m pub      # Pub/Sub only
cd bench && go run . -p 6379     # Against Redis for comparison
```

| Flag | Default | Description                  |
| ---- | ------- | ---------------------------- |
| `-p` | `8000`  | Server port                  |
| `-m` | `all`   | Mode: `all`, `key`, or `pub` |

## Configuration

FlashDB listens on `0.0.0.0:8000` by default. Edit `src/main.rs` to change:

- `PORT` — listening port
- `RDB_PATH` — snapshot file path (default: `flashdb.rdb`)
- `RDB_SAVE_INTERVAL` — auto-save interval (default: 300 seconds)

## Dependencies

| Crate             | Purpose                                   |
| ----------------- | ----------------------------------------- |
| `customhash`      | Lock-free concurrent hash map (workspace) |
| `crossbeam-utils` | CachePadded for false-sharing prevention  |
| `foldhash`        | Fast non-cryptographic hashing            |
| `mio`             | Non-blocking I/O, epoll wrapper           |
| `socket2`         | SO_REUSEPORT — per-thread kernel accept   |
| `memchr`          | SIMD-accelerated byte search (AVX2)       |
| `smallvec`        | Stack-allocated small vectors             |
| `mimalloc`        | High-performance memory allocator         |
| `num_cpus`        | CPU count for thread sizing               |
| `libc`            | signalfd, sigwait for graceful shutdown   |
| `crossbeam-queue` | Lock-free MPMC queue for pub/sub delivery |

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for a deep-dive into the lock-free hash map design, EBR algorithm, request lifecycle, and complexity analysis.
