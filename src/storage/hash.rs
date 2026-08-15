use crate::storage::store::Store;
use crate::storage::value::{FlashDB, StoreValue};
use crate::utils::util::format_float;
use std::collections::HashMap;

impl Store {
    pub fn hset(&self, key: &str, fields: Vec<(String, String)>) -> Result<usize, &'static str> {
        let result = self.data.update_with(key, |val| {
            if val.is_expired() {
                let mut h = HashMap::new();
                let added = fields.len();
                for (f, v) in fields.iter() {
                    h.insert(f.clone(), v.clone());
                }
                val.value = FlashDB::Hash(Box::new(h));
                val.expires_ms = 0;
                return Ok(added);
            }
            match val.value.as_hash_mut() {
                Some(h) => {
                    let mut added = 0;
                    for (f, v) in fields.iter() {
                        if !h.contains_key(f) {
                            added += 1;
                        }
                        h.insert(f.clone(), v.clone());
                    }
                    Ok(added)
                }
                None => Err("WRONGTYPE"),
            }
        });

        match result {
            Some(r) => r,
            None => {
                let mut h = HashMap::new();
                let added = fields.len();
                for (f, v) in fields {
                    h.insert(f, v);
                }
                self.data.insert(key.to_string(), StoreValue::hash(h));
                Ok(added)
            }
        }
    }

    pub fn hsetnx(&self, key: &str, field: &str, value: String) -> Result<bool, &'static str> {
        let result = self.data.update_with(key, |val| {
            if val.is_expired() {
                let mut h = HashMap::new();
                h.insert(field.to_string(), value.clone());
                val.value = FlashDB::Hash(Box::new(h));
                val.expires_ms = 0;
                return Ok(true);
            }
            match val.value.as_hash_mut() {
                Some(h) => {
                    if h.contains_key(field) {
                        Ok(false)
                    } else {
                        h.insert(field.to_string(), value.clone());
                        Ok(true)
                    }
                }
                None => Err("WRONGTYPE"),
            }
        });

        match result {
            Some(r) => r,
            None => {
                let mut h = HashMap::new();
                h.insert(field.to_string(), value);
                self.data.insert(key.to_string(), StoreValue::hash(h));
                Ok(true)
            }
        }
    }

    pub fn hget(&self, key: &str, field: &str) -> Result<Option<String>, &'static str> {
        match self.data.get_ref(key) {
            None => Ok(None),
            Some(e) if e.is_expired() => Ok(None),
            Some(e) => match e.value.as_hash() {
                Some(h) => Ok(h.get(field).cloned()),
                None => Err("WRONGTYPE"),
            },
        }
    }

    pub fn hmget(&self, key: &str, fields: &[&str]) -> Result<Vec<Option<String>>, &'static str> {
        match self.data.get_ref(key) {
            None => Ok(vec![None; fields.len()]),
            Some(e) if e.is_expired() => Ok(vec![None; fields.len()]),
            Some(e) => match e.value.as_hash() {
                Some(h) => Ok(fields.iter().map(|f| h.get(*f).cloned()).collect()),
                None => Err("WRONGTYPE"),
            },
        }
    }

    pub fn hgetall(&self, key: &str) -> Result<Vec<(String, String)>, &'static str> {
        match self.data.get_ref(key) {
            None => Ok(vec![]),
            Some(e) if e.is_expired() => Ok(vec![]),
            Some(e) => match e.value.as_hash() {
                Some(h) => Ok(h.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
                None => Err("WRONGTYPE"),
            },
        }
    }

    pub fn hdel(&self, key: &str, fields: &[&str]) -> Result<usize, &'static str> {
        let result = self.data.update_with(key, |val| {
            if val.is_expired() {
                return Ok(0);
            }
            match val.value.as_hash_mut() {
                Some(h) => {
                    let count = fields.iter().filter(|f| h.remove(**f).is_some()).count();
                    Ok(count)
                }
                None => Err("WRONGTYPE"),
            }
        });

        match result {
            Some(r) => r,
            None => Ok(0),
        }
    }

    pub fn hexists(&self, key: &str, field: &str) -> Result<bool, &'static str> {
        match self.data.get_ref(key) {
            None => Ok(false),
            Some(e) if e.is_expired() => Ok(false),
            Some(e) => match e.value.as_hash() {
                Some(h) => Ok(h.contains_key(field)),
                None => Err("WRONGTYPE"),
            },
        }
    }

    pub fn hlen(&self, key: &str) -> Result<usize, &'static str> {
        match self.data.get_ref(key) {
            None => Ok(0),
            Some(e) if e.is_expired() => Ok(0),
            Some(e) => match e.value.as_hash() {
                Some(h) => Ok(h.len()),
                None => Err("WRONGTYPE"),
            },
        }
    }

    pub fn hkeys(&self, key: &str) -> Result<Vec<String>, &'static str> {
        match self.data.get_ref(key) {
            None => Ok(vec![]),
            Some(e) if e.is_expired() => Ok(vec![]),
            Some(e) => match e.value.as_hash() {
                Some(h) => Ok(h.keys().cloned().collect()),
                None => Err("WRONGTYPE"),
            },
        }
    }

    pub fn hvals(&self, key: &str) -> Result<Vec<String>, &'static str> {
        match self.data.get_ref(key) {
            None => Ok(vec![]),
            Some(e) if e.is_expired() => Ok(vec![]),
            Some(e) => match e.value.as_hash() {
                Some(h) => Ok(h.values().cloned().collect()),
                None => Err("WRONGTYPE"),
            },
        }
    }

    pub fn hincrby(&self, key: &str, field: &str, by: i64) -> Result<i64, &'static str> {
        let result = self.data.update_with(key, |val| {
            if val.is_expired() {
                let mut h = HashMap::new();
                h.insert(field.to_string(), by.to_string());
                val.value = FlashDB::Hash(Box::new(h));
                val.expires_ms = 0;
                return Ok(by);
            }
            match val.value.as_hash_mut() {
                Some(h) => {
                    let n = h
                        .get(field)
                        .map(|v| v.as_str())
                        .unwrap_or("0")
                        .parse::<i64>();
                    match n {
                        Ok(n) => {
                            let Some(new) = n.checked_add(by) else {
                                return Err("increment or decrement would overflow");
                            };
                            h.insert(field.to_string(), new.to_string());
                            Ok(new)
                        }
                        Err(_) => Err("value is not an integer or out of range"),
                    }
                }
                None => Err("WRONGTYPE"),
            }
        });

        match result {
            Some(r) => r,
            None => {
                let mut h = HashMap::new();
                h.insert(field.to_string(), by.to_string());
                if self
                    .data
                    .insert_if_absent(key.to_string(), StoreValue::hash(h))
                {
                    Ok(by)
                } else {
                    self.hincrby(key, field, by)
                }
            }
        }
    }

    pub fn hincrbyfloat(&self, key: &str, field: &str, by: f64) -> Result<f64, &'static str> {
        let result = self.data.update_with(key, |val| {
            if val.is_expired() {
                let mut h = HashMap::new();
                h.insert(field.to_string(), format_float(by));
                val.value = FlashDB::Hash(Box::new(h));
                val.expires_ms = 0;
                return Ok(by);
            }
            match val.value.as_hash_mut() {
                Some(h) => {
                    let n = h
                        .get(field)
                        .map(|v| v.as_str())
                        .unwrap_or("0")
                        .parse::<f64>();
                    match n {
                        Ok(n) => {
                            let new = n + by;
                            h.insert(field.to_string(), format_float(new));
                            Ok(new)
                        }
                        Err(_) => Err("value is not a valid float"),
                    }
                }
                None => Err("WRONGTYPE"),
            }
        });

        match result {
            Some(r) => r,
            None => {
                let mut h = HashMap::new();
                h.insert(field.to_string(), format_float(by));
                self.data.insert(key.to_string(), StoreValue::hash(h));
                Ok(by)
            }
        }
    }
}
