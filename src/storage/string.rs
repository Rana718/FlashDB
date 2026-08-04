use crate::storage::{store::Store, value::StoreValue};
use crate::utils::util::format_float;

impl Store {
    pub fn set(&self, key: String, value: StoreValue) {
        self.data.insert(key, value);
    }

    /// Optimized SET from &str — avoids key allocation when key already exists.
    #[inline]
    pub fn set_string(&self, key: &str, value: &str, expires_ms: u64) {
        let store_val = StoreValue {
            value: crate::storage::value::FlashDB::String(value.to_owned()),
            expires_ms,
        };
        self.data.set(key, store_val, || key.to_owned());
    }

    pub fn get(&self, key: &str) -> Option<String> {
        // Hot path: single TLS access for pin+read+unpin
        let (h, idx) = self.data.locate_key(key);
        let result = self.data.with_entry(key, h, idx, |val| {
            if val.is_expired() {
                return None;
            }
            val.value.as_string().cloned()
        })?;
        if result.is_none() {
            // Expired — lazy delete
            self.data.remove(key);
        }
        result
    }

    /// Zero-copy GET: writes the value directly to output buffer without cloning.
    /// Returns false if key not found/expired.
    #[inline]
    pub fn get_to_buf(&self, key: &str, out: &mut Vec<u8>) -> bool {
        let (h, idx) = self.data.locate_key(key);
        let found = self.data.with_entry(key, h, idx, |val| {
            if val.is_expired() {
                return false;
            }
            match val.value.as_string() {
                Some(s) => {
                    crate::utils::resp::write_bulk(out, s);
                    true
                }
                None => false,
            }
        });
        match found {
            Some(true) => true,
            Some(false) => {
                // Expired
                self.data.remove(key);
                false
            }
            None => false,
        }
    }

    pub fn getdel(&self, key: &str) -> Option<String> {
        let entry = self.data.remove(key)?;
        if entry.is_expired() {
            return None;
        }
        entry.value.as_string().cloned()
    }

    pub fn getset(&self, key: String, new_value: String) -> Option<String> {
        let old = self.get(&key);
        self.data.insert(key, StoreValue::string(new_value));
        old
    }

    pub fn getex_ms(&self, key: &str, expires_ms: u64) -> Option<String> {
        self.data.try_update(key, |val| {
            if val.is_expired() {
                return None;
            }
            let s = val.value.as_string()?.clone();
            let mut new_val = val.clone();
            new_val.expires_ms = expires_ms;
            Some((new_val, s))
        })
    }

    pub fn setnx(&self, key: String, value: String) -> bool {
        // Check if key exists and is not expired
        if let Some(v) = self.data.get_ref(&key) {
            if !v.is_expired() {
                return false;
            }
            // Expired — remove and fall through to insert
            drop(v);
            self.data.remove(&key);
        }
        self.data.insert_if_absent(key, StoreValue::string(value))
    }

    pub fn append(&self, key: &str, suffix: &str) -> Result<usize, &'static str> {
        // Try to update existing
        let result = self.data.try_update(key, |val| {
            if val.is_expired() {
                // Treat as new key
                let new_val = StoreValue::string(suffix.to_string());
                return Some((new_val, Ok(suffix.len())));
            }
            match val.value.as_string() {
                Some(s) => {
                    let mut new_s = s.clone();
                    new_s.push_str(suffix);
                    let len = new_s.len();
                    Some((StoreValue::string(new_s), Ok(len)))
                }
                None => Some((val.clone(), Err("WRONGTYPE"))),
            }
        });

        match result {
            Some(r) => r,
            None => {
                // Key doesn't exist — insert new
                let len = suffix.len();
                self.data.insert(key.to_string(), StoreValue::string(suffix.to_string()));
                Ok(len)
            }
        }
    }

    pub fn strlen(&self, key: &str) -> Result<usize, &'static str> {
        match self.data.get_ref(key) {
            None => Ok(0),
            Some(e) if e.is_expired() => Ok(0),
            Some(e) => match e.value.as_string() {
                Some(s) => Ok(s.len()),
                None => Err("WRONGTYPE"),
            },
        }
    }

    pub fn getrange(&self, key: &str, start: i64, end: i64) -> String {
        let entry = match self.data.get_ref(key) {
            Some(e) if !e.is_expired() => e,
            _ => return String::new(),
        };
        let s = match entry.value.as_string() {
            Some(s) => s,
            None => return String::new(),
        };

        let len = s.len() as i64;
        if len == 0 {
            return String::new();
        }

        let start = if start < 0 {
            (len + start).max(0)
        } else {
            start.min(len)
        } as usize;
        let end = if end < 0 {
            (len + end).max(0)
        } else {
            end.min(len - 1)
        } as usize;

        if start > end {
            return String::new();
        }
        s[start..=end].to_string()
    }

    pub fn setrange(&self, key: &str, offset: usize, value: &str) -> Result<usize, &'static str> {
        let result = self.data.try_update(key, |val| {
            match val.value.as_string() {
                Some(s) => {
                    let mut bytes = s.clone().into_bytes();
                    let needed = offset + value.len();
                    if bytes.len() < needed {
                        bytes.resize(needed, 0u8);
                    }
                    bytes[offset..offset + value.len()].copy_from_slice(value.as_bytes());
                    let new_s = String::from_utf8_lossy(&bytes).into_owned();
                    let len = new_s.len();
                    let mut new_val = val.clone();
                    new_val.value = crate::storage::value::FlashDB::String(new_s);
                    Some((new_val, Ok(len)))
                }
                None => Some((val.clone(), Err("WRONGTYPE"))),
            }
        });

        match result {
            Some(r) => r,
            None => {
                // Key doesn't exist — create with zero padding
                let mut bytes = vec![0u8; offset + value.len()];
                bytes[offset..].copy_from_slice(value.as_bytes());
                let new_s = String::from_utf8_lossy(&bytes).into_owned();
                let len = new_s.len();
                self.data.insert(key.to_string(), StoreValue::string(new_s));
                Ok(len)
            }
        }
    }

    fn int_op(&self, key: &str, delta: i64) -> Result<i64, &'static str> {
        let result = self.data.try_update(key, |val| {
            match val.value.as_string() {
                Some(s) => {
                    let n = match s.parse::<i64>() {
                        Ok(n) => n,
                        Err(_) => return Some((val.clone(), Err("value is not an integer or out of range"))),
                    };
                    let new = n + delta;
                    let mut new_val = val.clone();
                    new_val.value = crate::storage::value::FlashDB::String(new.to_string());
                    Some((new_val, Ok(new)))
                }
                None => Some((val.clone(), Err("WRONGTYPE"))),
            }
        });

        match result {
            Some(r) => r,
            None => {
                // Key doesn't exist — create with delta as value
                self.data.insert(key.to_string(), StoreValue::string(delta.to_string()));
                Ok(delta)
            }
        }
    }

    pub fn incr(&self, key: &str) -> Result<i64, &'static str> {
        self.int_op(key, 1)
    }
    pub fn decr(&self, key: &str) -> Result<i64, &'static str> {
        self.int_op(key, -1)
    }
    pub fn incrby(&self, key: &str, by: i64) -> Result<i64, &'static str> {
        self.int_op(key, by)
    }
    pub fn decrby(&self, key: &str, by: i64) -> Result<i64, &'static str> {
        self.int_op(key, -by)
    }

    pub fn incrbyfloat(&self, key: &str, by: f64) -> Result<f64, &'static str> {
        let result = self.data.try_update(key, |val| {
            match val.value.as_string() {
                Some(s) => {
                    let n = match s.parse::<f64>() {
                        Ok(n) => n,
                        Err(_) => return Some((val.clone(), Err("value is not a valid float"))),
                    };
                    let new = n + by;
                    let mut new_val = val.clone();
                    new_val.value = crate::storage::value::FlashDB::String(format_float(new));
                    Some((new_val, Ok(new)))
                }
                None => Some((val.clone(), Err("WRONGTYPE"))),
            }
        });

        match result {
            Some(r) => r,
            None => {
                self.data.insert(key.to_string(), StoreValue::string(format_float(by)));
                Ok(by)
            }
        }
    }
}
