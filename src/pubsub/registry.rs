use std::sync::Arc;
use std::sync::atomic::{AtomicPtr, Ordering};

use foldhash::fast::RandomState;
use std::hash::BuildHasher;

use crate::utils::util::glob_match_bytes;

use super::frame::{encode_message, encode_pmessage};
use super::slot::SubSlot;

struct PatternEntry {
    pattern: String,
    slot: Arc<SubSlot>,
}

const CHANNEL_SHARDS: usize = 64;

struct ChannelData {
    name: String,
    slots: Vec<Arc<SubSlot>>,
}

struct ChannelShard {
    snapshot: AtomicPtr<Vec<ChannelData>>,
    mu: std::sync::Mutex<Vec<*mut Vec<ChannelData>>>,
}

unsafe impl Send for ChannelShard {}
unsafe impl Sync for ChannelShard {}

impl ChannelShard {
    fn new() -> Self {
        let empty: Vec<ChannelData> = Vec::new();
        Self {
            snapshot: AtomicPtr::new(Box::into_raw(Box::new(empty))),
            mu: std::sync::Mutex::new(Vec::new()),
        }
    }

    #[inline(always)]
    fn publish(&self, channel: &str, frame: &Arc<[u8]>) -> usize {
        let ptr = self.snapshot.load(Ordering::Acquire);
        if ptr.is_null() {
            return 0;
        }
        let channels = unsafe { &*ptr };
        for ch in channels {
            if ch.name == channel {
                let n = ch.slots.len();
                for slot in &ch.slots {
                    slot.push(Arc::clone(frame));
                }
                return n;
            }
        }
        0
    }

    fn subscribe(&self, channel: &str, slot: Arc<SubSlot>) {
        let mut retired = self.mu.lock().unwrap_or_else(|e| e.into_inner());
        let old_ptr = self.snapshot.load(Ordering::Acquire);
        let old = if old_ptr.is_null() {
            &[][..]
        } else {
            unsafe { &**old_ptr }
        };

        let mut new_vec: Vec<ChannelData> = Vec::with_capacity(old.len() + 1);
        let mut found = false;
        for ch in old {
            if ch.name == channel {
                let mut new_slots = ch.slots.clone();
                new_slots.push(slot.clone());
                new_vec.push(ChannelData {
                    name: ch.name.clone(),
                    slots: new_slots,
                });
                found = true;
            } else {
                new_vec.push(ChannelData {
                    name: ch.name.clone(),
                    slots: ch.slots.clone(),
                });
            }
        }
        if !found {
            new_vec.push(ChannelData {
                name: channel.to_string(),
                slots: vec![slot],
            });
        }

        let new_ptr = Box::into_raw(Box::new(new_vec));
        self.snapshot.store(new_ptr, Ordering::Release);
        if !old_ptr.is_null() {
            retired.push(old_ptr);
        }
        while retired.len() > 4 {
            let p = retired.remove(0);
            unsafe { drop(Box::from_raw(p)) };
        }
    }

    fn unsubscribe(&self, channel: &str, slot: &Arc<SubSlot>) {
        let mut retired = self.mu.lock().unwrap_or_else(|e| e.into_inner());
        let old_ptr = self.snapshot.load(Ordering::Acquire);
        if old_ptr.is_null() {
            return;
        }
        let old = unsafe { &*old_ptr };

        let mut new_vec: Vec<ChannelData> = Vec::with_capacity(old.len());
        for ch in old {
            if ch.name == channel {
                let new_slots: Vec<Arc<SubSlot>> = ch
                    .slots
                    .iter()
                    .filter(|s| !Arc::ptr_eq(s, slot))
                    .cloned()
                    .collect();
                if !new_slots.is_empty() {
                    new_vec.push(ChannelData {
                        name: ch.name.clone(),
                        slots: new_slots,
                    });
                }
            } else {
                new_vec.push(ChannelData {
                    name: ch.name.clone(),
                    slots: ch.slots.clone(),
                });
            }
        }

        let new_ptr = Box::into_raw(Box::new(new_vec));
        self.snapshot.store(new_ptr, Ordering::Release);
        retired.push(old_ptr);
        while retired.len() > 4 {
            let p = retired.remove(0);
            unsafe { drop(Box::from_raw(p)) };
        }
    }

    fn count_for(&self, channel: &str) -> usize {
        let ptr = self.snapshot.load(Ordering::Acquire);
        if ptr.is_null() {
            return 0;
        }
        let channels = unsafe { &*ptr };
        for ch in channels {
            if ch.name == channel {
                return ch.slots.len();
            }
        }
        0
    }

    fn active_channels(&self, pattern: Option<&str>) -> Vec<String> {
        let ptr = self.snapshot.load(Ordering::Acquire);
        if ptr.is_null() {
            return Vec::new();
        }
        let channels = unsafe { &*ptr };
        let mut result = Vec::new();
        for ch in channels {
            if !ch.slots.is_empty()
                && pattern.is_none_or(|p| glob_match_bytes(p.as_bytes(), ch.name.as_bytes()))
            {
                result.push(ch.name.clone());
            }
        }
        result
    }
}

impl Drop for ChannelShard {
    fn drop(&mut self) {
        let p = self.snapshot.load(Ordering::Relaxed);
        if !p.is_null() {
            unsafe { drop(Box::from_raw(p)) };
        }
        let retired = self.mu.get_mut().unwrap_or_else(|e| e.into_inner());
        for p in retired.drain(..) {
            unsafe { drop(Box::from_raw(p)) };
        }
    }
}

pub struct PubSub {
    shards: Box<[ChannelShard]>,
    patterns: std::sync::RwLock<Vec<PatternEntry>>,
    hasher: RandomState,
}

impl PubSub {
    pub fn new() -> Self {
        let shards = (0..CHANNEL_SHARDS)
            .map(|_| ChannelShard::new())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            shards,
            patterns: std::sync::RwLock::new(Vec::new()),
            hasher: RandomState::default(),
        }
    }

    #[inline(always)]
    fn shard_for(&self, channel: &str) -> &ChannelShard {
        let h = self.hasher.hash_one(channel) as usize;
        &self.shards[h % CHANNEL_SHARDS]
    }

    pub fn subscribe(&self, channel: &str, slot: Arc<SubSlot>) {
        self.shard_for(channel).subscribe(channel, slot);
    }

    pub fn unsubscribe(&self, channel: &str, slot: &Arc<SubSlot>) {
        self.shard_for(channel).unsubscribe(channel, slot);
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

    #[inline]
    pub fn publish(&self, channel: &str, message: &str) -> usize {
        let mut count = 0usize;

        let frame: Arc<[u8]> = encode_message(channel, message).into();
        count += self.shard_for(channel).publish(channel, &frame);

        let guard = self.patterns.read().unwrap_or_else(|e| e.into_inner());
        if !guard.is_empty() {
            let chan_b = channel.as_bytes();
            for entry in guard.iter() {
                if glob_match_bytes(entry.pattern.as_bytes(), chan_b) {
                    let pframe: Arc<[u8]> =
                        encode_pmessage(&entry.pattern, channel, message).into();
                    entry.slot.push(pframe);
                    count += 1;
                }
            }
        }

        count
    }

    pub fn active_channels(&self, pattern: Option<&str>) -> Vec<String> {
        let mut result = Vec::new();
        for shard in self.shards.iter() {
            result.extend(shard.active_channels(pattern));
        }
        result
    }

    pub fn numsub(&self, channels: &[&str]) -> Vec<(String, usize)> {
        channels
            .iter()
            .map(|&ch| {
                let n = self.shard_for(ch).count_for(ch);
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
