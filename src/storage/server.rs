use crate::storage::store::Store;
use tokio::time::Instant;

impl Store {
    pub fn cleanup_expired(&self) {
        let now = Instant::now();
        self.data.retain(|_, entry| entry.expires_at.map_or(true, |exp| exp > now));
    }

    pub fn info(&self) -> String {
        let total_keys = self.data.len();
        format!("# Store\r\ntotal_keys:{}\r\n", total_keys,)
    }

    pub fn flush(&self) {
        self.data.clear();
    }

    pub fn dbsize(&self) -> usize {
        self.data.len()
    }

    pub fn type_of(&self, key: &str) -> &'static str {
        match self.data.get(key) {
            Some(e) if !e.is_expired() => e.value.type_name(),
            _ => "none",
        }
    }
}
