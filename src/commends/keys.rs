use crate::utils::resp::{self, OK};
use crate::storage::store::Store;
use std::time::Duration;
use tokio::time::Instant;

pub async fn del(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, keys @ ..] if !keys.is_empty() => {
            resp::integer(keys.iter().filter(|k| store.del(k)).count() as i64)
        }
        _ => resp::wrong_args("del"),
    }
}

pub async fn unlink(parts: Vec<String>, store: &Store) -> String {
    del(parts, store).await
}

pub async fn exists_check(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, keys @ ..] if !keys.is_empty() => {
            resp::integer(keys.iter().filter(|k| store.exists(k)).count() as i64)
        }
        _ => resp::wrong_args("exists"),
    }
}

pub async fn ttl_check(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key] => match store.ttl(key) {
            Some(d) => resp::integer(d.as_secs() as i64),
            None => if store.exists(key) { ":-1\r\n".into() } else { ":-2\r\n".into() },
        },
        _ => resp::wrong_args("ttl"),
    }
}

pub async fn pttl_check(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key] => match store.pttl(key) {
            Some(d) => resp::integer(d.as_millis() as i64),
            None => if store.exists(key) { ":-1\r\n".into() } else { ":-2\r\n".into() },
        },
        _ => resp::wrong_args("pttl"),
    }
}

pub async fn expire(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key, secs] => match secs.parse::<u64>() {
            Ok(s) => resp::boolean(store.expire(key, Duration::from_secs(s))),
            Err(_) => resp::err("value is not an integer"),
        },
        _ => resp::wrong_args("expire"),
    }
}

pub async fn pexpire(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key, ms] => match ms.parse::<u64>() {
            Ok(m) => resp::boolean(store.expire(key, Duration::from_millis(m))),
            Err(_) => resp::err("value is not an integer"),
        },
        _ => resp::wrong_args("pexpire"),
    }
}

pub async fn expireat(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key, ts] => match ts.parse::<u64>() {
            Ok(unix) => {
                let now_unix = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let diff = if unix > now_unix {
                    Duration::from_secs(unix - now_unix)
                } else {
                    Duration::ZERO
                };
                resp::boolean(store.expireat(key, Instant::now() + diff))
            }
            Err(_) => resp::err("value is not an integer"),
        },
        _ => resp::wrong_args("expireat"),
    }
}

pub async fn persist(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key] => resp::boolean(store.persist(key)),
        _ => resp::wrong_args("persist"),
    }
}

pub async fn rename(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, old, new] => {
            if store.rename(old, new) { OK.into() } else { resp::err("no such key") }
        }
        _ => resp::wrong_args("rename"),
    }
}

pub async fn renamenx(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, old, new] => resp::boolean(store.renamenx(old, new)),
        _ => resp::wrong_args("renamenx"),
    }
}

pub async fn copy(parts: Vec<String>, store: &Store) -> String {
    let replace = parts.iter().any(|p| p.eq_ignore_ascii_case("REPLACE"));
    match parts.as_slice() {
        [_, src, dst, ..] => resp::boolean(store.copy(src, dst, replace)),
        _ => resp::wrong_args("copy"),
    }
}

pub async fn randomkey(_parts: Vec<String>, store: &Store) -> String {
    resp::opt_bulk(store.randomkey())
}

pub async fn keys(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, pattern] => {
            let mut all = store.keys_matching(pattern);
            all.sort();
            resp::array(&all)
        }
        _ => resp::wrong_args("keys"),
    }
}
