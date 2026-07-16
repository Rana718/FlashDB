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

            let key = match key.parse::<i32>() {
                Ok(k) => k,
                Err(_) => return "-ERR invalid key\r\n".into(),
            };

            store.set(
                key,
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
        [_, key] => {
            let key = match key.parse::<i32>() {
                Ok(k) => k,
                Err(_) => return "-ERR invalid key\r\n".into(),
            };

            match store.get(key) {
                Some(value) => format!("${}\r\n{}\r\n", value.len(), value),
                None => "$-1\r\n".into(),
            }
        }

        _ => "-ERR wrong number of arguments\r\n".into(),
    }
}
