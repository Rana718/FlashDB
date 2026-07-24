use std::collections::HashMap;

#[derive(Clone)]
pub enum FlashDB {
    String(String),
    Hash(HashMap<String, String>),
}

impl FlashDB {
    pub fn type_name(&self) -> &'static str {
        match self {
            FlashDB::String(_) => "string",
            FlashDB::Hash(_) => "hash",
        }
    }

    pub fn as_string(&self) -> Option<&String> {
        match self {
            FlashDB::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_string_mut(&mut self) -> Option<&mut String> {
        match self {
            FlashDB::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_hash(&self) -> Option<&HashMap<String, String>> {
        match self {
            FlashDB::Hash(h) => Some(h),
            _ => None,
        }
    }

    pub fn as_hash_mut(&mut self) -> Option<&mut HashMap<String, String>> {
        match self {
            FlashDB::Hash(h) => Some(h),
            _ => None,
        }
    }

    pub fn mem_size(&self) -> usize {
        match self {
            FlashDB::String(s) => s.len(),
            FlashDB::Hash(h) => h.iter().map(|(k, v)| k.len() + v.len()).sum(),
        }
    }
}

#[derive(Clone)]
pub struct StoreValue {
    pub value: FlashDB,
    pub expires_ms: u64,
}

static UNIX_MS_CACHE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[inline(always)]
pub fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[inline]
pub fn tick_clock() {
    UNIX_MS_CACHE.store(now_ms(), std::sync::atomic::Ordering::Relaxed);
}

#[inline(always)]
pub fn approx_now_ms() -> u64 {
    let cached = UNIX_MS_CACHE.load(std::sync::atomic::Ordering::Relaxed);
    if cached == 0 { now_ms() } else { cached }
}

impl StoreValue {
    #[inline]
    pub fn string(s: String) -> Self {
        Self {
            value: FlashDB::String(s),
            expires_ms: 0,
        }
    }

    #[inline]
    pub fn string_with_expiry(s: String, expires_at: Option<std::time::Instant>) -> Self {
        let expires_ms = match expires_at {
            None => 0,
            Some(exp) => {
                let remaining = exp.saturating_duration_since(std::time::Instant::now());
                now_ms() + remaining.as_millis() as u64
            }
        };
        Self {
            value: FlashDB::String(s),
            expires_ms,
        }
    }

    #[inline]
    pub fn hash(h: HashMap<String, String>) -> Self {
        Self {
            value: FlashDB::Hash(h),
            expires_ms: 0,
        }
    }

    #[inline(always)]
    pub fn is_expired(&self) -> bool {
        self.expires_ms != 0 && approx_now_ms() >= self.expires_ms
    }

    #[inline]
    pub fn is_expired_precise(&self) -> bool {
        self.expires_ms != 0 && now_ms() >= self.expires_ms
    }

    #[inline]
    pub fn ttl_ms(&self) -> Option<u64> {
        if self.expires_ms == 0 {
            return None;
        }
        let now = now_ms();
        if now >= self.expires_ms {
            None
        } else {
            Some(self.expires_ms - now)
        }
    }
}
