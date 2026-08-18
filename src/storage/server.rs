use crate::storage::store::{Store, rss_bytes};
use crate::storage::value::{now_ms, tick_clock};

impl Store {
    pub fn cleanup_expired(&self) {
        tick_clock();
        let now = now_ms();
        let generation = self.ttl_generation();
        let mut live_ttls = 0usize;
        self.data.retain(|_, entry| {
            if entry.expires_ms == 0 {
                true
            } else if entry.expires_ms > now {
                live_ttls += 1;
                true
            } else {
                false
            }
        });
        self.finish_ttl_scan(generation, live_ttls);
    }

    pub fn cleanup_expired_shard(
        &self,
        shard: usize,
        start_slot: usize,
        max_slots: usize,
    ) -> (usize, usize, usize) {
        if !self.has_ttl_keys() {
            return (0, start_slot, 0);
        }
        tick_clock();
        let now = now_ms();
        let mut live_ttls = 0usize;
        let (next_slot, capacity) = self
            .data
            .retain_shard_range(shard, start_slot, max_slots, |_, entry| {
                if entry.expires_ms != 0 && entry.expires_ms <= now {
                    self.sub_ttl();
                    false
                } else {
                    if entry.expires_ms != 0 {
                        live_ttls += 1;
                    }
                    true
                }
            })
            .unwrap_or((start_slot, 0));
        (live_ttls, next_slot, capacity)
    }

    pub fn info(&self) -> String {
        let total_keys = self.data.len();
        let connected = self.connected_clients();
        let rss = rss_bytes();
        let rss_human = format_bytes(rss);

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
             used_memory:{rss}\r\n\
             used_memory_human:{rss_human}\r\n\
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
        self.reset_ttl_count();
        customhash::force_collect();
        customhash::force_collect();
        customhash::force_collect();
        unsafe { libmimalloc_sys::mi_collect(true) };
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
