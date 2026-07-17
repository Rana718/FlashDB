# FlashDB

A Redis-compatible in-memory key-value store written in Rust. Speaks the RESP protocol so any Redis client works out of the box.

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
```

## Supported Commands

### Connection
| Command | Usage |
|---|---|
| `PING [msg]` | Returns PONG or echoes msg |
| `ECHO msg` | Returns msg as bulk string |
| `INFO` | Server stats |
| `DBSIZE` | Number of keys |
| `FLUSH` | Delete all keys |

### String
| Command | Usage |
|---|---|
| `SET key value [EX s] [PX ms] [NX] [XX] [GET]` | Set a key |
| `GET key` | Get a value |
| `GETDEL key` | Get and delete |
| `GETSET key value` | Get old, set new |
| `GETEX key [EX s / PX ms / PERSIST]` | Get and update TTL |
| `SETNX key value` | Set if not exists |
| `SETEX key seconds value` | Set with TTL |
| `PSETEX key ms value` | Set with TTL in milliseconds |
| `MSET key val [key val ...]` | Set multiple keys |
| `MSETNX key val [key val ...]` | Set multiple if none exist |
| `MGET key [key ...]` | Get multiple keys |
| `INCR key` | Increment integer |
| `DECR key` | Decrement integer |
| `INCRBY key n` | Increment by N |
| `DECRBY key n` | Decrement by N |
| `INCRBYFLOAT key n` | Increment by float |
| `APPEND key value` | Append to string |
| `STRLEN key` | String length |
| `GETRANGE key start end` | Substring |
| `SETRANGE key offset value` | Overwrite at offset |

### Keys
| Command | Usage |
|---|---|
| `DEL key [key ...]` | Delete keys |
| `UNLINK key [key ...]` | Async delete (same as DEL here) |
| `EXISTS key [key ...]` | Check existence |
| `TYPE key` | Returns string / hash / none |
| `TTL key` | TTL in seconds (-1 = no expiry, -2 = missing) |
| `PTTL key` | TTL in milliseconds |
| `EXPIRE key seconds` | Set TTL |
| `PEXPIRE key ms` | Set TTL in milliseconds |
| `EXPIREAT key unix` | Set TTL as unix timestamp |
| `PERSIST key` | Remove TTL |
| `RENAME old new` | Rename key |
| `RENAMENX old new` | Rename if new doesn't exist |
| `COPY src dst [REPLACE]` | Copy key |
| `RANDOMKEY` | Return a random key |
| `KEYS pattern` | All keys matching glob pattern |
| `SCAN cursor [MATCH pat] [COUNT n]` | Cursor-based iteration |

### Hash
| Command | Usage |
|---|---|
| `HSET key field value [field value ...]` | Set fields |
| `HSETNX key field value` | Set field if not exists |
| `HGET key field` | Get field |
| `HMGET key field [field ...]` | Get multiple fields |
| `HMSET key field value [...]` | Set multiple (deprecated alias) |
| `HGETALL key` | Get all field/value pairs |
| `HDEL key field [field ...]` | Delete fields |
| `HEXISTS key field` | Check field existence |
| `HLEN key` | Number of fields |
| `HKEYS key` | All field names |
| `HVALS key` | All field values |
| `HINCRBY key field n` | Increment integer field |
| `HINCRBYFLOAT key field n` | Increment float field |

## Running Tests

```bash
cargo test
cargo test -- --quiet   # clean output
```

## Benchmarking

```bash
cd bench
go run main.go
```

## Configuration

FlashDB listens on `0.0.0.0:8000` by default. No config file — edit `src/main.rs` to change the port.

## Dependencies

| Crate | Purpose |
|---|---|
| `tokio` | Async runtime, TCP, timers |
| `dashmap` | Lock-free concurrent hashmap |
