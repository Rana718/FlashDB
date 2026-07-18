use crate::utils::resp::{self, OK};
use crate::storage::store::Store;
use std::time::Duration;
use std::time::Instant;

pub fn del(parts: &[String], store: &Store) -> String {
    match parts {
        [_, keys @ ..] if !keys.is_empty() => {
            resp::integer(keys.iter().filter(|k| store.del(k)).count() as i64)
        }
        _ => resp::wrong_args("del"),
    }
}

pub fn unlink(parts: &[String], store: &Store) -> String {
    del(parts, store)
}

pub fn exists_check(parts: &[String], store: &Store) -> String {
    match parts {
        [_, keys @ ..] if !keys.is_empty() => {
            resp::integer(keys.iter().filter(|k| store.exists(k)).count() as i64)
        }
        _ => resp::wrong_args("exists"),
    }
}

pub fn ttl_check(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key] => match store.ttl(key) {
            Some(d) => resp::integer(d.as_secs() as i64),
            None => if store.exists(key) { ":-1\r\n".into() } else { ":-2\r\n".into() },
        },
        _ => resp::wrong_args("ttl"),
    }
}

pub fn pttl_check(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key] => match store.pttl(key) {
            Some(d) => resp::integer(d.as_millis() as i64),
            None => if store.exists(key) { ":-1\r\n".into() } else { ":-2\r\n".into() },
        },
        _ => resp::wrong_args("pttl"),
    }
}

pub fn expire(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key, secs] => match secs.parse::<u64>() {
            Ok(s) => resp::boolean(store.expire(key, Duration::from_secs(s))),
            Err(_) => resp::err("value is not an integer"),
        },
        _ => resp::wrong_args("expire"),
    }
}

pub fn pexpire(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key, ms] => match ms.parse::<u64>() {
            Ok(m) => resp::boolean(store.expire(key, Duration::from_millis(m))),
            Err(_) => resp::err("value is not an integer"),
        },
        _ => resp::wrong_args("pexpire"),
    }
}

pub fn expireat(parts: &[String], store: &Store) -> String {
    match parts {
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

pub fn persist(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key] => resp::boolean(store.persist(key)),
        _ => resp::wrong_args("persist"),
    }
}

pub fn rename(parts: &[String], store: &Store) -> String {
    match parts {
        [_, old, new] => {
            if store.rename(old, new) { OK.into() } else { resp::err("no such key") }
        }
        _ => resp::wrong_args("rename"),
    }
}

pub fn renamenx(parts: &[String], store: &Store) -> String {
    match parts {
        [_, old, new] => resp::boolean(store.renamenx(old, new)),
        _ => resp::wrong_args("renamenx"),
    }
}

pub fn copy(parts: &[String], store: &Store) -> String {
    let replace = parts.iter().any(|p| p.eq_ignore_ascii_case("REPLACE"));
    match parts {
        [_, src, dst, ..] => resp::boolean(store.copy(src, dst, replace)),
        _ => resp::wrong_args("copy"),
    }
}

pub fn randomkey(_parts: &[String], store: &Store) -> String {
    resp::opt_bulk(store.randomkey())
}

pub fn keys(parts: &[String], store: &Store) -> String {
    match parts {
        [_, pattern] => {
            let mut all = store.keys_matching(pattern);
            all.sort();
            resp::array(&all)
        }
        _ => resp::wrong_args("keys"),
    }
}
