use crate::storage::store::Store;
use std::time::Duration;
use tokio::time::Instant;

impl Store {
    pub fn del(&self, key: i32) -> bool {
        self.data.remove(&key).is_some()
    }

    pub fn exists(&self, key: i32) -> bool {
        self.data.contains_key(&key)
    }

    pub fn expire(&self, key: i32, duration: Duration) -> bool {
        let Some(mut data) = self.data.get_mut(&key) else {
            return false;
        };
        data.expires_at = Some(Instant::now() + duration);
        true
    }

    pub fn ttl(&self, key: i32) -> Option<Duration> {
        let data = self.data.get(&key)?;

        match data.expires_at {
            Some(exp) => Some(exp.saturating_duration_since(Instant::now())),
            None => None,
        }
    }

    pub fn incr(&self, key: i32) -> Option<i64> {
        let mut data = self.data.get_mut(&key)?;
        let value = data.value.parse::<i64>().ok()?;
        let new_value = value + 1;
        data.value = new_value.to_string();
        Some(new_value)
    }

    pub fn decr(&self, key: i32) -> Option<i64> {
        let mut data = self.data.get_mut(&key)?;
        let value = data.value.parse::<i64>().ok()?;
        let new_value = value - 1;
        data.value = new_value.to_string();
        Some(new_value)
    }
}
