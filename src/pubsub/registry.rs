use std::sync::Arc;

use foldhash::fast::RandomState;
use std::collections::HashMap;
use std::hash::BuildHasher;
use std::sync::RwLock;

use crate::utils::util::glob_match_bytes;

use super::frame::{encode_message, encode_pmessage};
use super::slot::SubSlot;

struct PatternEntry {
    pattern: String,
    slot: Arc<SubSlot>,
}

const CHANNEL_SHARDS: usize = 64;

struct ChannelShard {
    map: RwLock<HashMap<String, Vec<Arc<SubSlot>>>>,
}

impl ChannelShard {
    fn new() -> Self {
        Self {
            map: RwLock::new(HashMap::new()),
        }
    }
}

pub struct PubSub {
    shards: Box<[ChannelShard]>,
    patterns: RwLock<Vec<PatternEntry>>,
    hasher: RandomState,
}

impl Default for PubSub {
    fn default() -> Self {
        Self::new()
    }
}

impl PubSub {
    pub fn new() -> Self {
        let shards = (0..CHANNEL_SHARDS)
            .map(|_| ChannelShard::new())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            shards,
            patterns: RwLock::new(Vec::new()),
            hasher: RandomState::default(),
        }
    }

    #[inline(always)]
    fn shard_for(&self, channel: &str) -> &ChannelShard {
        // Phase G: Use foldhash for channel sharding (faster + better distribution)
        let h = self.hasher.hash_one(channel) as usize;
        &self.shards[h % CHANNEL_SHARDS]
    }

    pub fn subscribe(&self, channel: &str, slot: Arc<SubSlot>) {
        let shard = self.shard_for(channel);
        // Phase G: poison-safe — unwrap_or_else handles poisoned locks
        let mut map = shard.map.write().unwrap_or_else(|e| e.into_inner());
        map.entry(channel.to_string()).or_default().push(slot);
    }

    pub fn unsubscribe(&self, channel: &str, slot: &Arc<SubSlot>) {
        let shard = self.shard_for(channel);
        let mut map = shard.map.write().unwrap_or_else(|e| e.into_inner());
        if let Some(slots) = map.get_mut(channel) {
            slots.retain(|s| !Arc::ptr_eq(s, slot));
            if slots.is_empty() {
                map.remove(channel);
            }
        }
    }

    pub fn psubscribe(&self, pattern: &str, slot: Arc<SubSlot>) {
        let mut patterns = self.patterns.write().unwrap_or_else(|e| e.into_inner());
        patterns.push(PatternEntry {
            pattern: pattern.to_string(),
            slot,
        });
    }

    pub fn punsubscribe(&self, pattern: &str, slot: &Arc<SubSlot>) {
        let mut patterns = self.patterns.write().unwrap_or_else(|e| e.into_inner());
        patterns.retain(|e| !(e.pattern == pattern && Arc::ptr_eq(&e.slot, slot)));
    }

    pub fn publish(&self, channel: &str, message: &str) -> usize {
        let mut count = 0usize;

        let shard = self.shard_for(channel);
        let map = shard.map.read().unwrap_or_else(|e| e.into_inner());
        if let Some(slots) = map.get(channel) {
            let n = slots.len();
            if n > 0 {
                let frame: Arc<[u8]> = encode_message(channel, message).into();
                for slot in slots.iter() {
                    slot.push(Arc::clone(&frame));
                }
                count += n;
            }
        }
        drop(map);

        let guard = self.patterns.read().unwrap_or_else(|e| e.into_inner());
        if !guard.is_empty() {
            let chan_b = channel.as_bytes();
            for entry in guard.iter() {
                if glob_match_bytes(entry.pattern.as_bytes(), chan_b) {
                    let frame: Arc<[u8]> = encode_pmessage(&entry.pattern, channel, message).into();
                    entry.slot.push(frame);
                    count += 1;
                }
            }
        }

        count
    }

    pub fn active_channels(&self, pattern: Option<&str>) -> Vec<String> {
        let mut result = Vec::new();
        for shard in self.shards.iter() {
            let map = shard.map.read().unwrap_or_else(|e| e.into_inner());
            for (key, slots) in map.iter() {
                if !slots.is_empty()
                    && pattern.is_none_or(|p| glob_match_bytes(p.as_bytes(), key.as_bytes()))
                {
                    result.push(key.clone());
                }
            }
        }
        result
    }

    pub fn numsub(&self, channels: &[&str]) -> Vec<(String, usize)> {
        channels
            .iter()
            .map(|&ch| {
                let n = self
                    .shard_for(ch)
                    .map
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(ch)
                    .map(|s| s.len())
                    .unwrap_or(0);
                (ch.to_string(), n)
            })
            .collect()
    }

    pub fn numpat(&self) -> usize {
        self.patterns
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }
}
