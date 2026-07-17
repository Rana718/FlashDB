use crate::storage::{store::Store, value::StoreValue};
use tokio::time::Instant;

impl Store {
    pub fn set(&self, key: String, value: StoreValue) {
        self.data.insert(key, value);
    }

    pub fn get(&self, key: String) -> Option<String> {
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
}
