# FyroDB

> **Previously known as FlashDB** — renamed to FyroDB.

A Redis-compatible in-memory key-value store written in Rust. Speaks the RESP protocol so any Redis client works out of the box. Uses a fully lock-free concurrent hash map with epoch-based reclamation — no mutex, no RwLock on the data path.

Built on [`customhash`](https://www.ranadolui.me/blog/custom-concurrent-hashmap-rust) — a sharded, lock-free concurrent hash map with epoch-based reclamation written from scratch. Reads are wait-free, writes are lock-free, and retired values are freed only after all readers have unpinned. Read the full write-up: [Custom Concurrent HashMap in Rust](https://www.ranadolui.me/blog/custom-concurrent-hashmap-rust)

## Performance

Peak observed on a 6-core Intel i5-11400H (12 hardware threads), loopback TCP,
100 clients, 1M operations, and a warmed server. Each figure is the best of
three complete runs; sustained throughput will vary with CPU scheduling, cache
state, key cardinality, and subscriber fan-out.

| Metric           | FyroDB (6 cores) | Redis Cluster (6 nodes) | vs Cluster |
| ---------------- | ----------------- | ----------------------- | ---------- |
| Pipeline-64 SET  | ~14.7M ops/sec    | ~3.5M ops/sec           | 4.2x       |
| Pipeline-100 SET | ~14.9M ops/sec    | ~7.9M ops/sec           | 1.9x       |
| Pipeline-100 GET | ~19.3M ops/sec    | ~8.3M ops/sec           | 2.3x       |
| Pub/Sub delivery | ~36.8M msg/sec    | ~7.3M msg/sec           | 5.0x       |

> A single FyroDB node outperforms a 6-node Redis Cluster. Redis is single-threaded per node; FyroDB scales linearly with cores.

### Resource Usage

| Measurement             | Result  |
| ----------------------- | ------- |
| Idle RSS (no keys)      | ~55 MB          |
| Average RSS under load  | ~215 MB         |
| Peak RSS during a run   | ~235 MB         |
| Average CPU under load  | ~50%            |
| Peak CPU during a run   | ~60%            |

### Resource Comparison (FyroDB vs Redis Cluster during benchmark)

|          | FyroDB (1 node) | Redis Cluster (6 nodes) |
| -------- | ---------------- | ----------------------- |
| Idle RSS | ~55 MB           | ~75 MB (total)  |
| Peak RSS | ~235 MB          | ~154 MB (total) |
| Avg RSS  | ~215 MB          | ~126 MB (total) |
| Peak CPU | ~60%             | ~96%            |
| Avg CPU  | ~50%             | ~25%            |

> FyroDB uses more memory (pre-allocated lock-free hash table slots) but delivers 2–4x the throughput of a 6-node cluster on less CPU. The memory cost is the trade-off for zero-lock, zero-contention data access.

## Quick Start

```bash
cargo build --release
./target/release/fyro_db

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
docker run -p 8000:8000 rana718/fyrodb:latest
```

## Key Design Decisions

- **Lock-free CustomMap** — custom concurrent hash map with epoch-based reclamation. No locks on read or write path. Values swapped atomically, old values freed after grace period. Tombstones compacted during growth.
- **Thread-per-core** — one epoll loop per CPU core, SO_REUSEPORT for kernel-level connection distribution.
- **Zero-copy GET** — reads write directly from stored value to TCP buffer. No String clone.
- **Inline SET/GET fast path** — hot commands dispatched from raw RESP bytes without building intermediate arrays.
- **In-place mutation** — INCR, HSET, HDEL mutate a clone via `update_with()` + CAS, avoiding caller-side HashMap clones.
- **Value pooling** — reclaimed allocations recycled in thread-local pool, eliminating malloc on update path.
- **Batched writes** — all epoll events processed before flushing responses, reducing syscall count.
- **Arc-snapshot Pub/Sub** — publish path reads an Arc-cloned snapshot with zero locks, no use-after-free risk.
- **First-byte pattern index** — PSUBSCRIBE patterns bucketed by first character, PUBLISH only checks relevant bucket instead of all patterns.
- **Capacity enforcement** — `max_keys` limit enforced at insertion, returning OOM errors when full.
- **Per-slot RDB save** — snapshot iterates per-slot with brief EBR pins, allowing GC between slots.
- **Graceful shutdown** — SIGTERM/SIGINT triggers worker drain (flush pending writes) before RDB save and exit.

## Persistence

FyroDB uses RDB snapshots — the same model as Redis.

- **On startup** — loads `fyrodb.rdb` from the current directory if it exists
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

FyroDB is configured via environment variables. All settings have production-ready defaults.

| Variable               | Default       | Description                                   |
| ---------------------- | ------------- | --------------------------------------------- |
| `FYRODB_PORT`         | `8000`        | TCP listening port                            |
| `FYRODB_WORKERS`      | `0` (auto)    | Worker threads (0 = number of CPU cores)      |
| `FYRODB_SHARDS`       | `0` (auto)    | Hash map shards (0 = workers × 4, power of 2) |
| `FYRODB_MAX_KEYS`     | `1000000`     | Expected max keys (sizes the hash table)      |
| `FYRODB_MAX_CLIENTS`  | `10000`       | Max concurrent connections (rejects above)    |
| `FYRODB_RDB_PATH`     | `fyrodb.rdb` | Snapshot file path                            |
| `FYRODB_RDB_INTERVAL` | `300`         | Auto-save interval in seconds                 |

### Examples

```bash
# Default (1M keys, auto workers)
./fyro_db

# High-capacity (10M keys, custom port)
FYRODB_PORT=6379 FYRODB_MAX_KEYS=10000000 ./fyro_db

# Minimal memory (100k keys)
FYRODB_MAX_KEYS=100000 ./fyro_db

# Docker with custom settings
docker run -p 6379:6379 \
  -e FYRODB_PORT=6379 \
  -e FYRODB_MAX_KEYS=5000000 \
  -e FYRODB_RDB_INTERVAL=60 \
  -v ./data:/data \
  rana718/fyrodb:latest
```

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
