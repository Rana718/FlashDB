use crate::storage::store::Store;
use crate::utils::util::format_float;
use crate::{hash_read, hash_write};

impl Store {
    pub fn hset(&self, key: &str, fields: Vec<(String, String)>) -> Result<usize, &'static str> {
        hash_write!(self, key, |h| {
            let mut added = 0;
            for (f, v) in fields {
                if !h.contains_key(&f) {
                    added += 1;
                }
                h.insert(f, v);
            }
            added
        })
    }

    pub fn hsetnx(&self, key: &str, field: &str, value: String) -> Result<bool, &'static str> {
        hash_write!(self, key, |h| {
            if h.contains_key(field) {
                false
            } else {
                h.insert(field.to_string(), value);
                true
            }
        })
    }

    pub fn hget(&self, key: &str, field: &str) -> Result<Option<String>, &'static str> {
        hash_read!(self, key, None, |h| h.get(field).cloned())
    }

    pub fn hmget(&self, key: &str, fields: &[String]) -> Result<Vec<Option<String>>, &'static str> {
        hash_read!(self, key, vec![None; fields.len()], |h| {
            fields.iter().map(|f| h.get(f).cloned()).collect::<Vec<_>>()
        })
    }

    pub fn hgetall(&self, key: &str) -> Result<Vec<(String, String)>, &'static str> {
        hash_read!(self, key, vec![], |h| {
            h.iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<Vec<_>>()
        })
    }

    pub fn hdel(&self, key: &str, fields: &[String]) -> Result<usize, &'static str> {
        match self.data.get_mut(key) {
            None => Ok(0),
            Some(e) if e.is_expired() => Ok(0),
            Some(mut e) => match e.value.as_hash_mut() {
                Some(h) => Ok(fields.iter().filter(|f| h.remove(*f).is_some()).count()),
                None => Err("WRONGTYPE"),
            },
        }
    }

    pub fn hexists(&self, key: &str, field: &str) -> Result<bool, &'static str> {
        hash_read!(self, key, false, |h| h.contains_key(field))
    }

    pub fn hlen(&self, key: &str) -> Result<usize, &'static str> {
        hash_read!(self, key, 0usize, |h| h.len())
    }

    pub fn hkeys(&self, key: &str) -> Result<Vec<String>, &'static str> {
        hash_read!(self, key, vec![], |h| h.keys().cloned().collect::<Vec<_>>())
    }

    pub fn hvals(&self, key: &str) -> Result<Vec<String>, &'static str> {
        hash_read!(self, key, vec![], |h| h
            .values()
            .cloned()
            .collect::<Vec<_>>())
    }

    pub fn hincrby(&self, key: &str, field: &str, by: i64) -> Result<i64, &'static str> {
        let mut entry = self.data.entry(key.to_string()).or_insert_with(|| {
            crate::storage::value::StoreValue::hash(std::collections::HashMap::new())
        });
        match entry.value.as_hash_mut() {
            Some(h) => {
                let n = h
                    .get(field)
                    .map(|v| v.as_str())
                    .unwrap_or("0")
                    .parse::<i64>()
                    .map_err(|_| "value is not an integer or out of range")?;
                let new = n + by;
                h.insert(field.to_string(), new.to_string());
                Ok(new)
            }
            None => Err("WRONGTYPE"),
        }
    }

    pub fn hincrbyfloat(&self, key: &str, field: &str, by: f64) -> Result<f64, &'static str> {
        let mut entry = self.data.entry(key.to_string()).or_insert_with(|| {
            crate::storage::value::StoreValue::hash(std::collections::HashMap::new())
        });
        match entry.value.as_hash_mut() {
            Some(h) => {
                let n = h
                    .get(field)
                    .map(|v| v.as_str())
                    .unwrap_or("0")
                    .parse::<f64>()
                    .map_err(|_| "value is not a valid float")?;
                let new = n + by;
                h.insert(field.to_string(), format_float(new));
                Ok(new)
            }
            None => Err("WRONGTYPE"),
        }
    }
}
