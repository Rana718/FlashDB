use crate::storage::store::Store;
use crate::utils::util::glob_match;
use std::time::Duration;
use tokio::time::Instant;

impl Store {
    pub fn del(&self, key: &str) -> bool {
        self.data.remove(key).is_some()
    }

    pub fn exists(&self, key: &str) -> bool {
        match self.data.get(key) {
            Some(e) if !e.is_expired() => true,
            _ => false,
        }
    }

    pub fn expire(&self, key: &str, duration: Duration) -> bool {
        match self.data.get_mut(key) {
            Some(mut e) if !e.is_expired() => {
                e.expires_at = Some(Instant::now() + duration);
                true
            }
            _ => false,
        }
    }

    pub fn expireat(&self, key: &str, at: Instant) -> bool {
        match self.data.get_mut(key) {
            Some(mut e) if !e.is_expired() => {
                e.expires_at = Some(at);
                true
            }
            _ => false,
        }
    }

    pub fn persist(&self, key: &str) -> bool {
        match self.data.get_mut(key) {
            Some(mut e) if !e.is_expired() && e.expires_at.is_some() => {
                e.expires_at = None;
                true
            }
            _ => false,
        }
    }

    pub fn ttl(&self, key: &str) -> Option<Duration> {
        let data = self.data.get(key)?;
        if data.is_expired() {
            return None;
        }
        data.expires_at
            .map(|exp| exp.saturating_duration_since(Instant::now()))
    }

    pub fn pttl(&self, key: &str) -> Option<Duration> {
        self.ttl(key)
    }

    pub fn rename(&self, old_key: &str, new_key: &str) -> bool {
        match self.data.remove(old_key) {
            Some((_, entry)) if !entry.is_expired() => {
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
        let entry = match self.data.get(src) {
            Some(e) if !e.is_expired() => e.clone(),
            _ => return false,
        };
        if !replace && self.data.contains_key(dst) {
            return false;
        }
        self.data.insert(dst.to_string(), entry);
        true
    }

    pub fn randomkey(&self) -> Option<String> {
        self.data
            .iter()
            .find(|e| !e.value().is_expired())
            .map(|e| e.key().clone())
    }

    pub fn keys_matching(&self, pattern: &str) -> Vec<String> {
        self.data
            .iter()
            .filter(|e| !e.value().is_expired() && glob_match(pattern, e.key()))
            .map(|e| e.key().clone())
            .collect()
    }
}
