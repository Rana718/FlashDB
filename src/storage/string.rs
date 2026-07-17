use crate::storage::{store::Store, value::StoreValue};
use crate::utils::util::format_float;
use tokio::time::Instant;

impl Store {
    pub fn set(&self, key: String, value: StoreValue) {
        self.data.insert(key, value);
    }

    pub fn get(&self, key: &str) -> Option<String> {
        let data = self.data.get(key)?;
        if data.is_expired() {
            drop(data);
            self.data.remove(key);
            return None;
        }
        data.value.as_string().cloned()
    }

    pub fn getdel(&self, key: &str) -> Option<String> {
        let (_, entry) = self.data.remove(key)?;
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

    pub fn getex(&self, key: &str, expires_at: Option<Instant>) -> Option<String> {
        let mut data = self.data.get_mut(key)?;
        if data.is_expired() {
            drop(data);
            self.data.remove(key);
            return None;
        }
        let val = data.value.as_string()?.clone();
        data.expires_at = expires_at;
        Some(val)
    }

    pub fn setnx(&self, key: String, value: String) -> bool {
        use dashmap::mapref::entry::Entry;
        match self.data.entry(key) {
            Entry::Vacant(e) => {
                e.insert(StoreValue::string(value));
                true
            }
            Entry::Occupied(_) => false,
        }
    }

    pub fn append(&self, key: &str, suffix: &str) -> Result<usize, &'static str> {
        use dashmap::mapref::entry::Entry;
        match self.data.entry(key.to_string()) {
            Entry::Vacant(e) => {
                let len = suffix.len();
                e.insert(StoreValue::string(suffix.to_string()));
                Ok(len)
            }
            Entry::Occupied(mut e) => {
                if e.get().is_expired() {
                    let len = suffix.len();
                    e.insert(StoreValue::string(suffix.to_string()));
                    return Ok(len);
                }
                match e.get_mut().value.as_string_mut() {
                    Some(s) => {
                        s.push_str(suffix);
                        Ok(s.len())
                    }
                    None => Err("WRONGTYPE"),
                }
            }
        }
    }

    pub fn strlen(&self, key: &str) -> Result<usize, &'static str> {
        match self.data.get(key) {
            None => Ok(0),
            Some(e) if e.is_expired() => Ok(0),
            Some(e) => match e.value.as_string() {
                Some(s) => Ok(s.len()),
                None => Err("WRONGTYPE"),
            },
        }
    }

    pub fn getrange(&self, key: &str, start: i64, end: i64) -> String {
        let entry = match self.data.get(key) {
            Some(e) if !e.is_expired() => e,
            _ => return String::new(),
        };
        let s = match entry.value.as_string() {
            Some(s) => s.clone(),
            None => return String::new(),
        };
        drop(entry);

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
        use dashmap::mapref::entry::Entry;
        match self.data.entry(key.to_string()) {
            Entry::Vacant(e) => {
                let mut bytes = vec![0u8; offset + value.len()];
                bytes[offset..].copy_from_slice(value.as_bytes());
                let len = bytes.len();
                e.insert(StoreValue::string(
                    String::from_utf8_lossy(&bytes).into_owned(),
                ));
                Ok(len)
            }
            Entry::Occupied(mut e) => match e.get_mut().value.as_string_mut() {
                Some(s) => {
                    let mut bytes = std::mem::take(s).into_bytes();
                    let needed = offset + value.len();
                    if bytes.len() < needed {
                        bytes.resize(needed, 0u8);
                    }
                    bytes[offset..offset + value.len()].copy_from_slice(value.as_bytes());
                    *s = String::from_utf8_lossy(&bytes).into_owned();
                    Ok(s.len())
                }
                None => Err("WRONGTYPE"),
            },
        }
    }

    fn int_op(&self, key: &str, delta: i64) -> Result<i64, &'static str> {
        use dashmap::mapref::entry::Entry;
        match self.data.entry(key.to_string()) {
            Entry::Vacant(e) => {
                e.insert(StoreValue::string(delta.to_string()));
                Ok(delta)
            }
            Entry::Occupied(mut e) => match e.get_mut().value.as_string_mut() {
                Some(s) => {
                    let n = s
                        .parse::<i64>()
                        .map_err(|_| "value is not an integer or out of range")?;
                    let new = n + delta;
                    *s = new.to_string();
                    Ok(new)
                }
                None => Err("WRONGTYPE"),
            },
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
        use dashmap::mapref::entry::Entry;
        match self.data.entry(key.to_string()) {
            Entry::Vacant(e) => {
                e.insert(StoreValue::string(format_float(by)));
                Ok(by)
            }
            Entry::Occupied(mut e) => match e.get_mut().value.as_string_mut() {
                Some(s) => {
                    let n = s.parse::<f64>().map_err(|_| "value is not a valid float")?;
                    let new = n + by;
                    *s = format_float(new);
                    Ok(new)
                }
                None => Err("WRONGTYPE"),
            },
        }
    }
}
