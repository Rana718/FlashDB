use crate::storage::{store::Store, value::StoreValue};
use tokio::time::Instant;

impl Store {
    pub fn cleanup_expired(&self) {
        let now = Instant::now();
        self.data.retain(|_, entry| entry.expires_at.map_or(true, |exp| exp > now));
    }

    pub fn info(&self) -> String {
        let mut memory_bytes = 0usize;

        for item in self.data.iter() {
            memory_bytes += std::mem::size_of::<i32>();
            memory_bytes += item.value().value.len();
            memory_bytes += std::mem::size_of::<StoreValue>();
        }

        format!(
            "# Store\r\n\
           total_keys:{}\r\n\
           memory_usage:{} bytes\r\n",
            self.data.len(),
            memory_bytes
        )
    }

    pub fn flush(&self) {
        self.data.clear();
    }
}
