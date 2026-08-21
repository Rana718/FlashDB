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
| Pipeline-64 SET      | ~7.67M ops/sec  | ~4.20M ops/sec          | 1.8×    |
| Pipeline-100 SET     | ~20.91M ops/sec | ~4.90M ops/sec          | 4.3×    |
| Pipeline-100 GET     | ~22.04M ops/sec | ~6.14M ops/sec          | 3.6×    |
| Mixed SET/GET        | ~19.95M ops/sec | ~3.43M ops/sec          | 5.8×    |
| INCR (counters)      | ~30.91M ops/sec | ~4.61M ops/sec          | 6.7×    |
| HSET/HGET            | ~22.01M ops/sec | ~3.39M ops/sec          | 6.5×    |
| LPUSH/RPOP           | ~35.08M ops/sec | ~3.48M ops/sec          | 10.1×   |
| SADD                 | ~24.40M ops/sec | ~3.63M ops/sec          | 6.7×    |
| ZADD                 | ~4.10M ops/sec  | ~2.99M ops/sec          | 1.4×    |
| JSON.SET/GET         | ~13.15M ops/sec | ~1.63M ops/sec          | 8.1×    |
| SET+EXPIRE           | ~7.26M ops/sec  | ~1.67M ops/sec          | 4.3×    |
| Hot Key (contention) | ~7.38M ops/sec  | ~1.70M ops/sec          | 4.3×    |
| Pub/Sub delivery     | ~27.02M msg/sec | ~6.73M msg/sec          | 4.0×    |
| Producer/Consumer    | ~2.62M ops/sec  | ~872.6K ops/sec         | 3.0×    |

> **Note:** Pipeline-64 SET includes hash table growth from empty to 1M keys. On a pre-warmed server the throughput reaches ~17M ops/sec. See [Production Tips](https://fyrodb.vercel.app/docs/production-tips) for pre-warming guidance.

### Resource Usage

|          | FyroDB (1 node) | Redis Cluster (6 nodes) |
| -------- | --------------- | ----------------------- |
| Idle RSS | ~4 MB           | ~150 MB (total)         |
| Peak RSS | 247 MB          | 650 MB (total)          |
| Avg RSS  | 134 MB          | 463 MB (total)          |
| Peak CPU | 96%             | 390%                    |
| Avg CPU  | 61%             | 159%                    |

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
