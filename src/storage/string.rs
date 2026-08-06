use crate::storage::{store::Store, value::StoreValue};
use crate::utils::util::format_float;
use customhash::Full;

/// Hard cap for a single string value (matches the RESP parser's bulk limit).
const MAX_STRING_BYTES: usize = 512 * 1024 * 1024;

impl Store {
    pub fn set(&self, key: String, value: StoreValue) {
        self.data.insert(key, value);
    }

    /// Fallible set — returns Err(Full) when the store is at capacity.
    pub fn try_set_value(&self, key: String, value: StoreValue) -> Result<(), Full> {
        self.data.try_insert(key, value)?;
        Ok(())
    }

    #[inline]
    pub fn set_string(&self, key: &str, value: &str, expires_ms: u64) {
        let store_val = StoreValue {
            value: crate::storage::value::FlashDB::String(value.to_owned()),
            expires_ms,
        };
        self.data.set(key, store_val, || key.to_owned());
    }

    /// Fallible set_string — returns Err(Full) on OOM.
    #[inline]
    pub fn try_set_string(&self, key: &str, value: &str, expires_ms: u64) -> Result<(), Full> {
        let store_val = StoreValue {
            value: crate::storage::value::FlashDB::String(value.to_owned()),
            expires_ms,
        };
        self.data.try_set(key, store_val, || key.to_owned())?;
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<String> {
        let (h, idx) = self.data.locate_key(key);
        let result = self.data.with_entry(key, h, idx, |val| {
            if val.is_expired() {
                return Err(());
            }
            Ok(val.value.as_string().cloned())
        })?;
        match result {
            Err(()) => {
                self.data.remove(key);
                None
            }
            Ok(v) => v,
        }
    }

    #[inline]
    pub fn get_to_buf(&self, key: &str, out: &mut Vec<u8>) -> bool {
        let (h, idx) = self.data.locate_key(key);
        let found = self.data.with_entry(key, h, idx, |val| {
            if val.is_expired() {
                return Err(());
            }
            match val.value.as_string() {
                Some(s) => {
                    crate::utils::resp::write_bulk(out, s);
                    Ok(true)
                }
                None => Ok(false),
            }
        });
        match found {
            Some(Ok(true)) => true,
            Some(Err(())) => {
                self.data.remove(key);
                false
            }
            _ => false,
        }
    }

    pub fn getdel(&self, key: &str) -> Option<String> {
        let entry = self.data.remove(key)?;
        if entry.is_expired() {
            return None;
        }
        entry.value.as_string().cloned()
    }

    /// Atomic GETSET: retrieves old value and sets new value atomically via CAS.
    pub fn getset(&self, key: &str, new_value: &str) -> Option<String> {
        let nv = new_value.to_string();
        // Try update existing key
        let result = self.data.try_update(key, |val| {
            let old = if val.is_expired() {
                None
            } else {
                val.value.as_string().cloned()
            };
            Some((StoreValue::string(nv.clone()), old))
        });
        match result {
            Some(old) => old,
            None => {
                // Key doesn't exist — insert new
                self.data.insert(key.to_string(), StoreValue::string(new_value.to_string()));
                None
            }
        }
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
        if let Some(replaced) = self.data.try_update(&key, |current| {
            if current.is_expired() {
                Some((StoreValue::string(value.clone()), true))
            } else {
                Some((current.clone(), false))
            }
        }) {
            return replaced;
        }
        self.data.insert_if_absent(key, StoreValue::string(value))
    }

    pub fn append(&self, key: &str, suffix: &str) -> Result<usize, &'static str> {
        let result = self.data.try_update(key, |val| {
            if val.is_expired() {
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
                let len = suffix.len();
                self.data
                    .insert(key.to_string(), StoreValue::string(suffix.to_string()));
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

        let mut start = if start < 0 {
            (len + start).max(0)
        } else {
            start.min(len)
        } as usize;
        let mut end = if end < 0 {
            (len + end).max(0)
        } else {
            end.min(len - 1)
        } as usize;

        if start > end {
            return String::new();
        }

        // Indices are byte offsets and may land inside a multi-byte char.
        // Expand outward to char boundaries so the result stays valid UTF-8.
        while start > 0 && !s.is_char_boundary(start) {
            start -= 1;
        }
        end += 1;
        while end < s.len() && !s.is_char_boundary(end) {
            end += 1;
        }
        s[start..end].to_string()
    }

    pub fn setrange(&self, key: &str, offset: usize, value: &str) -> Result<usize, &'static str> {
        let Some(needed) = offset.checked_add(value.len()) else {
            return Err("offset is out of range");
        };
        if needed > MAX_STRING_BYTES {
            return Err("string exceeds maximum allowed size");
        }

        let result = self
            .data
            .try_update(key, |val| match val.value.as_string() {
                Some(s) => {
                    let mut bytes = s.clone().into_bytes();
                    if bytes.len() < needed {
                        bytes.resize(needed, 0u8);
                    }
                    bytes[offset..offset + value.len()].copy_from_slice(value.as_bytes());
                    match String::from_utf8(bytes) {
                        Ok(new_s) => {
                            let len = new_s.len();
                            let mut new_val = val.clone();
                            new_val.value = crate::storage::value::FlashDB::String(new_s);
                            Some((new_val, Ok(len)))
                        }
                        Err(_) => Some((val.clone(), Err("result would not be valid UTF-8"))),
                    }
                }
                None => Some((val.clone(), Err("WRONGTYPE"))),
            });

        match result {
            Some(r) => r,
            None => {
                let mut bytes = vec![0u8; needed];
                bytes[offset..].copy_from_slice(value.as_bytes());
                let new_s = match String::from_utf8(bytes) {
                    Ok(s) => s,
                    Err(_) => return Err("result would not be valid UTF-8"),
                };
                let len = new_s.len();
                if self
                    .data
                    .insert_if_absent(key.to_string(), StoreValue::string(new_s))
                {
                    Ok(len)
                } else {
                    self.setrange(key, offset, value)
                }
            }
        }
    }

    fn int_op(&self, key: &str, delta: i64) -> Result<i64, &'static str> {
        let result = self
            .data
            .try_update(key, |val| match val.value.as_string() {
                Some(s) => {
                    let n = match s.parse::<i64>() {
                        Ok(n) => n,
                        Err(_) => {
                            return Some((
                                val.clone(),
                                Err("value is not an integer or out of range"),
                            ));
                        }
                    };
                    let Some(new) = n.checked_add(delta) else {
                        return Some((val.clone(), Err("increment or decrement would overflow")));
                    };
                    let mut new_val = val.clone();
                    new_val.value = crate::storage::value::FlashDB::String(new.to_string());
                    Some((new_val, Ok(new)))
                }
                None => Some((val.clone(), Err("WRONGTYPE"))),
            });

        match result {
            Some(r) => r,
            None => {
                if self
                    .data
                    .insert_if_absent(key.to_string(), StoreValue::string(delta.to_string()))
                {
                    Ok(delta)
                } else {
                    self.int_op(key, delta)
                }
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
        let delta = by
            .checked_neg()
            .ok_or("increment or decrement would overflow")?;
        self.int_op(key, delta)
    }

    pub fn incrbyfloat(&self, key: &str, by: f64) -> Result<f64, &'static str> {
        let result = self
            .data
            .try_update(key, |val| match val.value.as_string() {
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
            });

        match result {
            Some(r) => r,
            None => {
                self.data
                    .insert(key.to_string(), StoreValue::string(format_float(by)));
                Ok(by)
            }
        }
    }
}
