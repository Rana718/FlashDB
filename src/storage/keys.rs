use crate::storage::store::Store;
use crate::storage::value::now_ms;
use crate::utils::util::glob_match;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Thread-local xorshift64 PRNG for RANDOMKEY reservoir sampling.
static RNG_STATE: AtomicU64 = AtomicU64::new(0);

#[inline]
fn fast_rand(bound: u64) -> u64 {
    let mut s = RNG_STATE.load(Ordering::Relaxed);
    if s == 0 {
        // Seed from current time
        s = now_ms() ^ 0x517cc1b727220a95;
    }
    s ^= s << 13;
    s ^= s >> 7;
    s ^= s << 17;
    RNG_STATE.store(s, Ordering::Relaxed);
    s % bound
}

impl Store {
    pub fn del(&self, key: &str) -> bool {
        self.data.remove(key).is_some()
    }

    pub fn exists(&self, key: &str) -> bool {
        matches!(self.data.get_ref(key), Some(e) if !e.is_expired_precise())
    }

    pub fn expire(&self, key: &str, duration: Duration) -> bool {
        self.data
            .try_update(key, |val| {
                if val.is_expired() {
                    return None;
                }
                let mut new_val = val.clone();
                new_val.expires_ms = now_ms() + duration.as_millis() as u64;
                Some((new_val, true))
            })
            .unwrap_or(false)
    }

    pub fn expire_ms(&self, key: &str, abs_ms: u64) -> bool {
        self.data
            .try_update(key, |val| {
                if val.is_expired() {
                    return None;
                }
                let mut new_val = val.clone();
                new_val.expires_ms = abs_ms;
                Some((new_val, true))
            })
            .unwrap_or(false)
    }

    pub fn persist(&self, key: &str) -> bool {
        self.data
            .try_update(key, |val| {
                if val.is_expired() || val.expires_ms == 0 {
                    return None;
                }
                let mut new_val = val.clone();
                new_val.expires_ms = 0;
                Some((new_val, true))
            })
            .unwrap_or(false)
    }

    pub fn ttl(&self, key: &str) -> Option<Duration> {
        let data = self.data.get_ref(key)?;
        if data.is_expired() {
            return None;
        }
        data.ttl_ms().map(Duration::from_millis)
    }

    pub fn pttl(&self, key: &str) -> Option<Duration> {
        self.ttl(key)
    }

    pub fn ttl_value_ms(&self, key: &str) -> i64 {
        let data = match self.data.get_ref(key) {
            Some(data) => data,
            None => return -2,
        };
        if data.is_expired() {
            return -2;
        }
        data.ttl_ms()
            .map_or(-1, |ms| ms.min(i64::MAX as u64) as i64)
    }

    pub fn rename(&self, old_key: &str, new_key: &str) -> bool {
        if old_key == new_key {
            return self.exists(old_key);
        }
        match self.data.remove(old_key) {
            Some(entry) if !entry.is_expired() => {
                self.data.insert(new_key.to_string(), entry);
                true
            }
            _ => false,
        }
    }

    pub fn renamenx(&self, old_key: &str, new_key: &str) -> bool {
        if self.data.contains_key(new_key) {
            return false;
        }
        self.rename(old_key, new_key)
    }

    pub fn copy(&self, src: &str, dst: &str, replace: bool) -> bool {
        let vref = match self.data.get_ref(src) {
            Some(e) if !e.is_expired() => e,
            _ => return false,
        };
        let entry: crate::storage::value::StoreValue = (*vref).clone();
        drop(vref);
        if replace {
            self.data.insert(dst.to_string(), entry);
            true
        } else {
            self.data.insert_if_absent(dst.to_string(), entry)
        }
    }

    pub fn randomkey(&self) -> Option<String> {
        // True random: collect all live keys, pick one at random.
        // For performance, we do a single pass and use reservoir sampling (k=1).
        let mut result: Option<String> = None;
        let mut count: u64 = 0;
        self.data.for_each(|key, val| {
            if !val.is_expired() {
                count += 1;
                // Simple xorshift-based probability: pick this key with probability 1/count
                // Use a fast RNG seeded from the key address and count
                if count == 1 || fast_rand(count) == 0 {
                    result = Some(key.to_string());
                }
            }
        });
        result
    }

    pub fn keys_matching(&self, pattern: &str) -> Vec<String> {
        let mut keys = Vec::new();
        self.data.for_each(|key, val| {
            if !val.is_expired() && glob_match(pattern, key) {
                keys.push(key.to_string());
            }
        });
        keys
    }
}
