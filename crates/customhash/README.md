# customhash

`customhash` is a fixed-capacity, sharded concurrent hash map specialized for
low-latency string workloads. Lookups are wait-free under the fixed-capacity
invariant; inserts and value replacement are lock-free. Values are published by
atomic pointer replacement and reclaimed with epoch-based reclamation.

The optimized API is:

```rust
use customhash::CustomMap;

let map = CustomMap::with_capacity(64, 1_000_000);
map.insert("key".to_owned(), "value".to_owned());
let key = map.prepare("key");
let reader = map.read();
assert_eq!(reader.get_prepared("key", key), Some("value"));
```

The map has fixed capacity and does not physically reuse slots for unrelated
deleted keys. It is designed for pre-sized, high-read string maps.

## Guarantees and limitations

`get`, `contains_key`, and prepared reads are wait-free while the map remains
within its configured capacity. Inserts, updates, and removes are lock-free,
but may retry under contention. `remove` is a logical deletion: the key slot
is retained so readers never observe freed entry memory, and the slot is
reusable only by reinserting the same key. Size the map for the maximum key
population and handle `try_insert` returning `Full` when capacity is reached.

The crate deliberately targets `String -> String`. Redis values such as hashes,
expiry metadata, and pub/sub subscriber lists need mutable, heterogeneous
storage and are not safely representable through this API without a separate
generic snapshot/copy-on-write design.
