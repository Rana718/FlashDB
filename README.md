# FyroDB

> **Previously known as FlashDB** — renamed to FyroDB.

## Changelog

**v0.1.2** — Memory-focused storage and allocator release. See the [v0.1.2 changelog](https://fyrodb.vercel.app/docs/changelog-0-1-2).

---

A Redis-compatible in-memory key-value store written in Rust. Speaks the RESP protocol so any Redis client works out of the box. Supports all major Redis data types and commands with a lock-free concurrent architecture.

Built on [`customhash`](https://www.ranadolui.me/blog/custom-concurrent-hashmap-rust) — a sharded, lock-free concurrent hash map with epoch-based reclamation and per-key seqlock for safe in-place mutation.

## Performance

6-core Intel i5-11400H (12 hardware threads), loopback TCP, 100 clients, 1M operations per benchmark.

| Benchmark            | FyroDB (1 node) | Redis Cluster (6 nodes) | Speedup |
| -------------------- | --------------- | ----------------------- | ------- |
| Pipeline-64 SET      | ~15.78M ops/sec | ~3.5M ops/sec           | 4.5×    |
| Pipeline-100 SET     | ~17.43M ops/sec | ~7.9M ops/sec           | 2.2×    |
| Pipeline-100 GET     | ~21.14M ops/sec | ~8.3M ops/sec           | 2.5×    |
| Mixed SET/GET        | ~21.04M ops/sec | ~3.11M ops/sec          | 6.8×    |
| INCR (counters)      | ~29.66M ops/sec | ~3.96M ops/sec          | 7.5×    |
| HSET/HGET            | ~20.62M ops/sec | ~3.52M ops/sec          | 5.9×    |
| LPUSH/RPOP           | ~37.60M ops/sec | ~3.64M ops/sec          | 10.3×   |
| SADD                 | ~22.57M ops/sec | ~2.84M ops/sec          | 7.9×    |
| ZADD                 | ~14.69M ops/sec | ~2.30M ops/sec          | 6.4×    |
| JSON.SET/GET         | ~6.87M ops/sec  | ~1.69M ops/sec          | 4.1×    |
| SET+EXPIRE           | ~13.19M ops/sec | ~1.85M ops/sec          | 7.1×    |
| Hot Key (contention) | ~27.26M ops/sec | ~1.65M ops/sec          | 16.5×   |
| Pub/Sub delivery     | ~30.52M msg/sec | ~6.03M msg/sec          | 5.1×    |
| Producer/Consumer    | ~2.71M ops/sec  | ~833.6K ops/sec         | 3.3×    |

### Resource Usage

|          | FyroDB (1 node) | Redis Cluster (6 nodes) |
| -------- | --------------- | ----------------------- |
| Idle RSS | ~4 MB           | ~150 MB (total)         |
| Peak RSS | 700 MB          | 829 MB (total)          |
| Avg RSS  | 464 MB          | 711 MB (total)          |
| Peak CPU | 88%             | 425%                    |
| Avg CPU  | 60%             | 199%                    |

A single FyroDB node outperforms a 6-node Redis Cluster on every workload while using less total CPU and comparable memory.

## Quick Start

```bash
cargo build --release
./target/release/fyro_db

redis-cli -p 8000
127.0.0.1:8000> SET name rana
OK
127.0.0.1:8000> GET name
"rana"
```

## Docker

```bash
docker run -p 8000:8000 rana718/fyrodb:latest
```

## Supported Data Types

- **String** — GET, SET, INCR, APPEND, GETRANGE, MSET, MGET, LCS, and more
- **Hash** — HSET, HGET, HGETALL, HINCRBY, HRANDFIELD, HSCAN
- **List** — LPUSH, RPUSH, LPOP, RPOP, LRANGE, LMOVE, BLPOP, BRPOP
- **Set** — SADD, SREM, SMEMBERS, SINTER, SUNION, SDIFF, SSCAN
- **Sorted Set** — ZADD, ZRANGE, ZRANGEBYSCORE, ZPOPMIN, ZUNIONSTORE, ZSCAN
- **JSON** — JSON.SET, JSON.GET, JSON.DEL, JSON.ARRAPPEND, JSON.OBJKEYS
- **Stream** — XADD, XREAD, XRANGE, XGROUP, XACK, XTRIM
- **Bitmap** — SETBIT, GETBIT, BITCOUNT, BITOP, BITFIELD
- **HyperLogLog** — PFADD, PFCOUNT, PFMERGE
- **Geospatial** — GEOADD, GEODIST, GEOSEARCH, GEOSEARCHSTORE

## Supported Commands

Full Redis command compatibility including:

**Keys:** DEL, UNLINK, EXISTS, TTL, PTTL, EXPIRE, PEXPIRE, EXPIREAT, EXPIRETIME, PEXPIRETIME, PERSIST, RENAME, RENAMENX, COPY, RANDOMKEY, KEYS, SCAN, TOUCH, OBJECT, SORT, TYPE

**Server:** PING, ECHO, INFO, DBSIZE, BGSAVE, SAVE, LASTSAVE, TIME, COMMAND, HELLO, SELECT, AUTH, QUIT, RESET, CLIENT, CONFIG, FLUSHALL, FLUSHDB, SLOWLOG, ACL

**Pub/Sub:** SUBSCRIBE, UNSUBSCRIBE, PSUBSCRIBE, PUNSUBSCRIBE, PUBLISH, PUBSUB

**Transactions:** MULTI, EXEC, DISCARD, WATCH, UNWATCH

## Design

- **Lock-free reads** — epoch-based reclamation with seqlock validation for iteration safety
- **Zero-clone writes** — per-key spinlock with in-place mutation, no CAS retry loops
- **Thread-per-core** — one epoll loop per CPU core, SO_REUSEPORT for kernel-level connection distribution
- **Zero-copy GET** — writes directly from stored value to TCP buffer
- **Inline fast path** — SET, GET, INCR, LPUSH, RPOP, SADD, DEL dispatched from raw RESP bytes
- **Compact storage** — short keys and values stay inline; small hashes, lists, and sets avoid full hash-table overhead
- **Adaptive memory reclaim** — lazy shard growth, EBR collection, allocator purging, shard compaction, and value defragmentation
- **Batched I/O** — all epoll events processed before flushing, reducing syscall count
- **Arc-snapshot Pub/Sub** — publish path reads with zero locks, per-subscriber lock-free queues

## Persistence

RDB snapshots, same model as Redis:

- Loads `fyrodb.rdb` on startup
- Auto-saves every 5 minutes (configurable)
- Saves on SIGTERM/Ctrl+C
- `BGSAVE` for manual trigger
- Atomic write (temp file → fsync → rename)

## Benchmarking

```bash
cd bench && go run .                 # Full benchmark
cd bench && go run . -m key          # KV only
cd bench && go run . -m pub          # Pub/Sub only
cd bench && go run . -m mix          # Mixed workload only
cd bench && go run . -p 6379         # Against Redis
```

| Flag        | Default | Description                       |
| ----------- | ------- | --------------------------------- |
| `-p`        | `8000`  | Server port                       |
| `-m`        | `all`   | Mode: `all`, `key`, `pub`, `mix`  |
| `--cluster` |         | Comma-separated cluster addresses |

## Configuration

| Variable              | Default      | Description                       |
| --------------------- | ------------ | --------------------------------- |
| `FYRODB_PORT`         | `8000`       | TCP listening port                |
| `FYRODB_BIND`         | `0.0.0.0`   | Bind address                      |
| `FYRODB_WORKERS`      | `0` (auto)   | Worker threads (0 = CPU cores)    |
| `FYRODB_SHARDS`       | `0` (auto)   | Hash map shards (0 = workers × 4) |
| `FYRODB_MAX_KEYS`     | `1000`       | Maximum live keys; override for larger deployments |
| `FYRODB_MAX_CLIENTS`  | `10000`      | Max concurrent connections        |
| `FYRODB_AUTH`         | (none)       | Password for AUTH (empty = no auth) |
| `FYRODB_RDB_PATH`     | `fyrodb.rdb` | Snapshot file path                |
| `FYRODB_RDB_INTERVAL` | `300`        | Auto-save interval in seconds     |

## Architecture
 
See [ARCHITECTURE.md](ARCHITECTURE.md) for the current memory layout, concurrency model, maintenance threads, and complexity reference.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines on development, benchmarks, zero-regression policy, and submitting pull requests.

## License

Apache 2.0
