use crate::storage::{store::Store, value::StoreValue};
use std::time::Duration;
use tokio::time::Instant;

pub async fn set(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key, value, rest @ ..] => {
            let expires_at = match rest {
                [] => None,

                [ttl_cmd, sec] if ttl_cmd.eq_ignore_ascii_case("EX") => sec
                    .parse::<u64>()
                    .ok()
                    .map(|s| Instant::now() + Duration::from_secs(s)),

                _ => return "-ERR invalid arguments\r\n".into(),
            };

            store.set(
                key.clone(),
                StoreValue {
                    value: value.clone(),
                    expires_at,
                },
            );

            "+OK\r\n".into()
        }

        _ => "-ERR wrong number of arguments\r\n".into(),
    }
}

pub async fn get(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key] => match store.get(key.clone()) {
            Some(value) => format!("${}\r\n{}\r\n", value.len(), value),
            None => "$-1\r\n".into(),
        },

        _ => "-ERR wrong number of arguments\r\n".into(),
    }
}

pub async fn incr(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key] => {
            let value = store.incr(key.clone());
            format!(":{}\r\n", value.unwrap_or(0))
        }

        _ => "-ERR wrong number of arguments\r\n".into(),
    }
}

pub async fn decr(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key] => {
            let value = store.decr(key.clone());
            format!(":{}\r\n", value.unwrap_or(0))
        }

        _ => "-ERR wrong number of arguments\r\n".into(),
    }
}

pub async fn mset(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, items @ ..] => {
            if items.is_empty() || items.len() % 2 != 0 {
                return "-ERR wrong number of arguments\r\n".into();
            }
            for item in items.chunks(2) {
                store.set(
                    item[0].clone(),
                    StoreValue {
                        value: item[1].clone(),
                        expires_at: None,
                    },
                );
            }
            "+OK\r\n".into()
        }

        _ => "-ERR wrong number of arguments\r\n".into(),
    }
}

pub async fn mget(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, keys @ ..] => {
            if keys.is_empty() {
                return "-ERR wrong number of arguments\r\n".into();
            }
            let mut response = format!("*{}\r\n", keys.len());
            for key in keys {
                match store.get(key.clone()) {
                    Some(value) => {
                        response.push_str(&format!("${}\r\n{}\r\n", value.len(), value));
                    }
                    None => {
                        response.push_str("$-1\r\n");
                    }
                }
            }
            response
        }

        _ => "-ERR wrong number of arguments\r\n".into(),
    }
}
