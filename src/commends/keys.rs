use crate::{storage::store::Store};

pub async fn del(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key] => {
            let key = match key.parse::<i32>() {
                Ok(k) => k,
                Err(_) => return "-ERR invalid key\r\n".into(),
            };

            let removed = store.del(key);

            format!(":{}\r\n", if removed { 1 } else { 0 })
        }

        _ => "-ERR wrong number of arguments\r\n".into(),
    }
}

pub async fn ttl_check(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key] => {
            let key = match key.parse::<i32>() {
                Ok(k) => k,
                Err(_) => return "-ERR invalid key\r\n".into(),
            };

            let ttl = store.ttl(key);
            format!(":{}\r\n", ttl.map_or(0, |d| d.as_secs() as i64))
        }

        _ => "-ERR wrong number of arguments\r\n".into(),
    }
}

pub async fn exists_check(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key] => {
            let key = match key.parse::<i32>() {
                Ok(k) => k,
                Err(_) => return "-ERR invalid key\r\n".into(),
            };

            let exists = store.exists(key);
            format!(":{}\r\n", if exists { 1 } else { 0 })
        }

        _ => "-ERR wrong number of arguments\r\n".into(),
    }
}