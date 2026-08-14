# customhash

`customhash` is a growable, sharded concurrent hash map specialized for
low-latency string workloads. Lookups are wait-free; value replacement is
lock-free, while new-key insertion briefly coordinates with the rare resize
path. Values and replaced slot tables are reclaimed with epoch-based
reclamation.

The optimized API is:

```rust
use customhash::CustomMap;

let map = CustomMap::with_capacity(64, 1_000_000);
map.insert("key".to_owned(), "value".to_owned());
let key = map.prepare("key");
let reader = map.read();
assert_eq!(reader.get_prepared("key", key), Some("value"));
```

The map does not physically reuse slots for unrelated deleted keys. It is
designed for pre-sized, high-read string maps, but grows when needed.

## Guarantees and limitations

`get` and `contains_key` are wait-free. Updates and removes are lock-free and
may retry under contention. New-key inserts coordinate with table growth so
migration cannot miss a concurrently published entry. `remove` is a logical deletion: the key slot
is retained so readers never observe freed entry memory, and the slot is
reusable only by reinserting the same key. Slot tables grow automatically and
old tables are freed after all readers that could reference them have unpinned.

The crate deliberately targets `String -> String`. Redis values such as hashes,
expiry metadata, and pub/sub subscriber lists need mutable, heterogeneous
storage and are not safely representable through this API without a separate
generic snapshot/copy-on-write design.
