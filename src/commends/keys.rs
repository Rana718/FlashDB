use std::time::Duration;

use crate::storage::store::Store;

pub async fn del(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key] => {
            let removed = store.del(key.clone());
            format!(":{}\r\n", if removed { 1 } else { 0 })
        }

        _ => "-ERR wrong number of arguments\r\n".into(),
    }
}

pub async fn ttl_check(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key] => {
            let ttl = store.ttl(key.clone());
            match ttl {
                Some(d) => format!(":{}\r\n", d.as_secs() as i64),
                None => {
                    if store.exists(key.clone()) {
                        ":-1\r\n".into()
                    } else {
                        ":-2\r\n".into()
                    }
                }
            }
        }

        _ => "-ERR wrong number of arguments\r\n".into(),
    }
}

pub async fn exists_check(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key] => {
            let exists = store.exists(key.clone());
            format!(":{}\r\n", if exists { 1 } else { 0 })
        }

        _ => "-ERR wrong number of arguments\r\n".into(),
    }
}

pub async fn expire(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key, duration] => {
            let duration = match duration.parse::<u64>() {
                Ok(d) => Duration::from_secs(d),
                Err(_) => return "-ERR invalid duration\r\n".into(),
            };

            let expired = store.expire(key.clone(), duration);
            format!(":{}\r\n", if expired { 1 } else { 0 })
        }

        _ => "-ERR wrong number of arguments\r\n".into(),
    }
}