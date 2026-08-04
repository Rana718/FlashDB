use crate::storage::store::Store;
use crate::storage::value::now_ms;
use crate::utils::util::glob_match;
use std::time::Duration;

impl Store {
    pub fn del(&self, key: &str) -> bool {
        self.data.remove(key).is_some()
    }

    pub fn exists(&self, key: &str) -> bool {
        match self.data.get_ref(key) {
            Some(e) if !e.is_expired() => true,
            _ => false,
        }
    }

    pub fn expire(&self, key: &str, duration: Duration) -> bool {
        self.data.try_update(key, |val| {
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
        self.data.try_update(key, |val| {
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
        self.data.try_update(key, |val| {
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
        data.ttl_ms().map(|ms| Duration::from_millis(ms))
    }

    pub fn pttl(&self, key: &str) -> Option<Duration> {
        self.ttl(key)
    }

    pub fn rename(&self, old_key: &str, new_key: &str) -> bool {
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
        if !replace && self.data.contains_key(dst) {
            return false;
        }
        let entry: crate::storage::value::StoreValue = (*vref).clone();
        drop(vref);
        self.data.insert(dst.to_string(), entry);
        true
    }

    pub fn randomkey(&self) -> Option<String> {
        let mut result = None;
        self.data.for_each(|key, val| {
            if result.is_none() && !val.is_expired() {
                result = Some(key.to_string());
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
