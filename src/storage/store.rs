use super::value::StoreValue;
use dashmap::{DashMap};
use tokio::time::{Duration, Instant};

#[derive(Clone)]
pub struct Store {
    data: DashMap<i32, StoreValue>,
}


impl Store {
    pub fn new() -> Self {
        Self {
            data: DashMap::new(),
        }
    }

    pub fn set(&self, key: i32, value: StoreValue) {
        self.data.insert(key, value);
    }

    pub fn get(&self, key: i32) -> Option<String> {
        let data = self.data.get(&key)?;

        if let Some(exp) = data.expires_at {
            if Instant::now() >= exp {
                drop(data);
                self.data.remove(&key);
                return None;
            }
        }

        Some(data.value.clone())
    }

    pub fn del(&self, key: i32) -> bool {
        self.data.remove(&key).is_some()
    }

    pub fn exists(&self, key: i32) -> bool {
        self.data.contains_key(&key)
    }

    pub fn ttl(&self, key: i32) -> Option<Duration> {
        let data = self.data.get(&key)?;

        match data.expires_at {
            Some(exp) => Some(exp.saturating_duration_since(Instant::now())),
            None => None,
        }
    }

    pub fn cleanup_expired(&self) {
        let now = Instant::now();
        self.data.retain(|_, entry| entry.expires_at.map_or(true, |exp| exp > now));
    }
}
