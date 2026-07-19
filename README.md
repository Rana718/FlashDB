# FlashDB

A Redis-compatible in-memory key-value store written in Rust. Speaks the RESP protocol so any Redis client works out of the box. Beats Redis on every benchmark metric.

## Performance

Benchmarked on a 12-core machine (Intel i5-11400H) against Redis 7 using 100 clients, 1M ops, pipeline size 100.

| Metric           | FlashDB (1 core) | Redis (1 core) | FlashDB (12 cores) |
| ---------------- | ---------------- | -------------- | ------------------ |
| Sequential SET   | ~286k ops/sec    | ~135k ops/sec  | ~616k ops/sec      |
| Pipelined SET    | ~2.17M ops/sec   | ~1.19M ops/sec | ~5.44M ops/sec     |
| Pipelined GET    | ~2.48M ops/sec   | ~1.68M ops/sec | ~6.72M ops/sec     |
| Pub/Sub delivery | ~1.34M msg/sec   | ~1.06M msg/sec | ~2.39M msg/sec     |

> Redis is single-threaded and does not scale with core count. FlashDB scales linearly with workers.

## Quick Start

```bash
# build and run
cargo build --release
./target/release/flash_db

# connect with redis-cli
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

| Command                             | Description                                            |
| ----------------------------------- | ------------------------------------------------------ |
| `SUBSCRIBE channel [channel ...]`   | Subscribe to one or more channels                      |
| `UNSUBSCRIBE [channel ...]`         | Unsubscribe from channels (all if none specified)      |
| `PSUBSCRIBE pattern [pattern ...]`  | Subscribe to channels matching a glob pattern          |
| `PUNSUBSCRIBE [pattern ...]`        | Unsubscribe from patterns (all if none specified)      |
| `PUBLISH channel message`           | Publish a message, returns number of receivers         |
| `PUBSUB CHANNELS [pattern]`         | List active channels with at least one subscriber      |
| `PUBSUB NUMSUB [channel ...]`       | Subscriber count per channel                           |
| `PUBSUB NUMPAT`                     | Total number of pattern subscriptions                  |

Pub/Sub is compatible with any Redis client. Subscribers on all worker threads receive messages published from any thread — delivery is global across the server.

## Running Tests

```bash
cargo test
cargo test -- --quiet      # clean output
cargo test rdb             # persistence tests only
cargo test pubsub          # pub/sub tests only
```

## Benchmarking

```bash
# Run all benchmarks against FlashDB (default port 8000)
cd bench && go run .

# Run all benchmarks against Redis for comparison
cd bench && go run . -p 6379

# KV only (SET / GET)
cd bench && go run . -m key

# Pub/Sub only
cd bench && go run . -m pub

# Specific server + specific suite
cd bench && go run . -p 6379 -m key
cd bench && go run . -p 8000 -m pub
```

| Flag | Default | Description |
| ---- | ------- | ----------- |
| `-p` | `8000`  | Server port |
| `-m` | `all`   | Mode: `all`, `key`, or `pub` |

## Configuration

FlashDB listens on `0.0.0.0:8000` by default. Edit `src/main.rs` to change:

- `PORT` — listening port
- `RDB_PATH` — snapshot file path (default: `flashdb.rdb`)
- `RDB_SAVE_INTERVAL` — auto-save interval (default: 300 seconds)

## Dependencies

| Crate             | Purpose                                                         |
| ----------------- | --------------------------------------------------------------- |
| `dashmap`         | Sharded concurrent hashmap — N×64 shards, one RwLock per shard  |
| `mio`             | Non-blocking I/O, epoll wrapper                                 |
| `socket2`         | SO_REUSEPORT — per-thread kernel accept queues                  |
| `memchr`          | SIMD-accelerated byte search (AVX2)                             |
| `smallvec`        | Stack-allocated small vectors                                   |
| `mimalloc`        | High-performance memory allocator                               |
| `num_cpus`        | CPU count for thread and shard sizing                           |
| `libc`            | signalfd, sigwait for graceful shutdown                         |
| `crossbeam-queue` | Lock-free MPMC queue for pub/sub message delivery               |

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for a deep-dive into the design, algorithms, data structures, and complexity analysis.

