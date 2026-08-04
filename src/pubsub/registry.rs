use std::collections::HashMap;
use std::sync::{Arc, RwLock};

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
        }
    }

    #[inline(always)]
    fn shard_for(&self, channel: &str) -> &ChannelShard {
        let h = fxhash(channel.as_bytes());
        &self.shards[h % CHANNEL_SHARDS]
    }

    pub fn subscribe(&self, channel: &str, slot: Arc<SubSlot>) {
        self.shard_for(channel)
            .map
            .write()
            .unwrap()
            .entry(channel.to_string())
            .or_default()
            .push(slot);
    }

    pub fn unsubscribe(&self, channel: &str, slot: &Arc<SubSlot>) {
        let mut map = self.shard_for(channel).map.write().unwrap();
        if let Some(slots) = map.get_mut(channel) {
            slots.retain(|s| !Arc::ptr_eq(s, slot));
            if slots.is_empty() {
                map.remove(channel);
            }
        }
    }

    pub fn psubscribe(&self, pattern: &str, slot: Arc<SubSlot>) {
        self.patterns.write().unwrap().push(PatternEntry {
            pattern: pattern.to_string(),
            slot,
        });
    }

    pub fn punsubscribe(&self, pattern: &str, slot: &Arc<SubSlot>) {
        self.patterns
            .write()
            .unwrap()
            .retain(|e| !(e.pattern == pattern && Arc::ptr_eq(&e.slot, slot)));
    }

    pub fn publish(&self, channel: &str, message: &str) -> usize {
        let mut count = 0usize;

        let map = self.shard_for(channel).map.read().unwrap();
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

        let guard = self.patterns.read().unwrap();
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
            let map = shard.map.read().unwrap();
            for (key, slots) in map.iter() {
                if !slots.is_empty()
                    && pattern.map_or(true, |p| glob_match_bytes(p.as_bytes(), key.as_bytes()))
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
                    .unwrap()
                    .get(ch)
                    .map(|s| s.len())
                    .unwrap_or(0);
                (ch.to_string(), n)
            })
            .collect()
    }

    pub fn numpat(&self) -> usize {
        self.patterns.read().unwrap().len()
    }
}

#[inline(always)]
fn fxhash(bytes: &[u8]) -> usize {
    let mut hash: usize = 0;
    for &b in bytes {
        hash = hash.wrapping_mul(0x01000193) ^ (b as usize);
    }
    hash
}
