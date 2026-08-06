use crate::storage::store::Store;
use crate::utils::util::glob_match;

impl Store {
    /// Real SCAN cursor implementation.
    /// The cursor encodes (shard_index << 20) | slot_index, giving a stable
    /// iteration position even as keys are added or removed.
    /// cursor=0 starts from the beginning; returned cursor=0 means iteration complete.
    pub fn scan(&self, cursor: usize, pattern: Option<&str>, count: usize) -> (usize, Vec<String>) {
        let mut keys = Vec::with_capacity(count);
        let total_shards = self.shard_count();

        if total_shards == 0 {
            return (0, keys);
        }

        // Decode cursor: upper bits = shard, lower 20 bits = slot within shard
        let (mut shard_idx, mut slot_idx) = if cursor == 0 {
            (0usize, 0usize)
        } else {
            ((cursor >> 20) & 0xFFF, cursor & 0xF_FFFF)
        };

        // Iterate through shards/slots until we've found `count` keys or exhausted all slots
        while shard_idx < total_shards {
            let shard_len = self.shard_slot_count(shard_idx);

            while slot_idx < shard_len {
                if let Some((key, val)) = self.peek_slot(shard_idx, slot_idx)
                    && !val.is_expired()
                    && pattern.is_none_or(|p| glob_match(p, &key))
                {
                    keys.push(key);
                }
                slot_idx += 1;

                // Once we have enough keys, return cursor pointing to next position
                if keys.len() >= count {
                    // Point cursor to next slot
                    if slot_idx >= shard_len {
                        shard_idx += 1;
                        slot_idx = 0;
                    }
                    if shard_idx >= total_shards {
                        return (0, keys);
                    }
                    let next_cursor = (shard_idx << 20) | (slot_idx & 0xF_FFFF);
                    return (next_cursor, keys);
                }
            }

            shard_idx += 1;
            slot_idx = 0;
        }

        // Full iteration complete
        (0, keys)
    }

    /// Number of shards in the underlying map.
    #[inline]
    fn shard_count(&self) -> usize {
        self.data.shard_count()
    }

    /// Number of slots in a specific shard.
    #[inline]
    fn shard_slot_count(&self, shard: usize) -> usize {
        self.data.shard_slot_count(shard)
    }

    /// Peek at a specific slot in a shard — returns (key, value) if occupied and live.
    #[inline]
    fn peek_slot(
        &self,
        shard: usize,
        slot: usize,
    ) -> Option<(String, crate::storage::value::StoreValue)> {
        self.data.peek_slot(shard, slot)
    }
}
