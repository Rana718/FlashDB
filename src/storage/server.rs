use crate::storage::store::{Store, data_memory_bytes, rss_bytes};
use crate::storage::value::{now_ms, tick_clock};

impl Store {
    pub fn cleanup_expired(&self) {
        tick_clock();
        let now = now_ms();
        self.data.retain(|_, entry| {
            entry.expires_ms == 0 || entry.expires_ms > now
        });
    }

    pub fn cleanup_expired_shard(&self, shard: usize) {
        tick_clock();
        let now = now_ms();
        self.data.retain_shard(shard, |_, entry| {
            entry.expires_ms == 0 || entry.expires_ms > now
        });
    }

    pub fn info(&self) -> String {
        let total_keys = self.data.len();
        let connected = self.connected_clients();
        let rss = rss_bytes();
        let rss_human = format_bytes(rss);
        let data_mem = data_memory_bytes();
        let data_human = format_bytes(data_mem);

        format!(
            "# Server\r\n\
             fyrodb_version:0.1.1\r\n\
             os:{os}\r\n\
             arch:{arch}\r\n\
             \r\n\
             # Clients\r\n\
             connected_clients:{connected}\r\n\
             \r\n\
             # Memory\r\n\
             used_memory:{data_mem}\r\n\
             used_memory_human:{data_human}\r\n\
             used_memory_rss:{rss}\r\n\
             used_memory_rss_human:{rss_human}\r\n\
             used_memory_peak:{rss}\r\n\
             used_memory_peak_human:{rss_human}\r\n\
             \r\n\
             # Stats\r\n\
             total_keys:{total_keys}\r\n",
            os = std::env::consts::OS,
            arch = std::env::consts::ARCH,
        )
    }

    pub fn flush(&self) {
        self.data.clear();
    }

    pub fn dbsize(&self) -> usize {
        self.data.len()
    }

    pub fn type_of(&self, key: &str) -> &'static str {
        match self.data.get_ref(key) {
            Some(e) if !e.is_expired_precise() => e.value.type_name(),
            _ => "none",
        }
    }
}

fn format_bytes(bytes: usize) -> String {
    const KB: usize = 1024;
    const MB: usize = KB * 1024;
    const GB: usize = MB * 1024;

    if bytes >= GB {
        format!("{:.2}G", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2}M", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2}K", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}
