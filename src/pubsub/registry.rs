use dashmap::DashMap;
use std::sync::{Arc, RwLock};

use crate::utils::util::glob_match_bytes;

use super::frame::{encode_message, encode_pmessage};
use super::slot::SubSlot;

struct PatternEntry {
    pattern: String,
    slot: Arc<SubSlot>,
}

pub struct PubSub {
    channels: DashMap<String, Vec<Arc<SubSlot>>>,
    patterns: RwLock<Vec<PatternEntry>>,
}

impl PubSub {
    pub fn new() -> Self {
        Self {
            channels: DashMap::new(),
            patterns: RwLock::new(Vec::new()),
        }
    }

    pub fn subscribe(&self, channel: &str, slot: Arc<SubSlot>) {
        self.channels
            .entry(channel.to_string())
            .or_default()
            .push(slot);
    }

    pub fn unsubscribe(&self, channel: &str, slot: &Arc<SubSlot>) {
        let mut entry = match self.channels.get_mut(channel) {
            Some(e) => e,
            None => return,
        };
        entry.retain(|s| !Arc::ptr_eq(s, slot));
        let empty = entry.is_empty();
        drop(entry);
        if empty {
            self.channels.remove_if(channel, |_, v| v.is_empty());
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

        if let Some(slots) = self.channels.get(channel) {
            let n = slots.len();
            if n > 0 {
                let frame = Arc::new(encode_message(channel, message));
                for slot in slots.iter() {
                    slot.push(Arc::clone(&frame));
                }
                count += n;
            }
        }

        {
            let guard = self.patterns.read().unwrap();
            if !guard.is_empty() {
                let chan_b = channel.as_bytes();
                for entry in guard.iter() {
                    if glob_match_bytes(entry.pattern.as_bytes(), chan_b) {
                        let frame = Arc::new(encode_pmessage(&entry.pattern, channel, message));
                        entry.slot.push(frame);
                        count += 1;
                    }
                }
            }
        }

        count
    }

    pub fn active_channels(&self, pattern: Option<&str>) -> Vec<String> {
        self.channels
            .iter()
            .filter(|e| {
                !e.value().is_empty()
                    && pattern.map_or(true, |p| glob_match_bytes(p.as_bytes(), e.key().as_bytes()))
            })
            .map(|e| e.key().clone())
            .collect()
    }

    pub fn numsub(&self, channels: &[&str]) -> Vec<(String, usize)> {
        channels
            .iter()
            .map(|&ch| {
                let n = self.channels.get(ch).map(|e| e.len()).unwrap_or(0);
                (ch.to_string(), n)
            })
            .collect()
    }

    pub fn numpat(&self) -> usize {
        self.patterns.read().unwrap().len()
    }
}
