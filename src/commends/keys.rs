use crate::parse_int;
use crate::storage::store::Store;
use crate::utils::resp;
use std::time::{Duration, Instant};

pub fn del(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, keys @ ..] if !keys.is_empty() => {
            resp::write_integer(out, keys.iter().filter(|k| store.del(k)).count() as i64)
        }
        _ => resp::write_wrong_args(out, "del"),
    }
}

pub fn unlink(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    del(parts, store, out);
}

pub fn exists_check(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, keys @ ..] if !keys.is_empty() => {
            resp::write_integer(out, keys.iter().filter(|k| store.exists(k)).count() as i64)
        }
        _ => resp::write_wrong_args(out, "exists"),
    }
}

pub fn ttl_check(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    let [_, key] = parts else {
        return resp::write_wrong_args(out, "ttl");
    };
    match store.ttl(key) {
        Some(d) => resp::write_integer(out, d.as_secs() as i64),
        None => out.extend_from_slice(if store.exists(key) {
            b":-1\r\n"
        } else {
            b":-2\r\n"
        }),
    }
}

pub fn pttl_check(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    let [_, key] = parts else {
        return resp::write_wrong_args(out, "pttl");
    };
    match store.pttl(key) {
        Some(d) => resp::write_integer(out, d.as_millis() as i64),
        None => out.extend_from_slice(if store.exists(key) {
            b":-1\r\n"
        } else {
            b":-2\r\n"
        }),
    }
}

pub fn expire(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    let [_, key, secs] = parts else {
        return resp::write_wrong_args(out, "expire");
    };
    let s = parse_int!(out, secs.as_str(), u64);
    resp::write_boolean(out, store.expire(key, Duration::from_secs(s)));
}

pub fn pexpire(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    let [_, key, ms] = parts else {
        return resp::write_wrong_args(out, "pexpire");
    };
    let m = parse_int!(out, ms.as_str(), u64);
    resp::write_boolean(out, store.expire(key, Duration::from_millis(m)));
}

pub fn expireat(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    let [_, key, ts] = parts else {
        return resp::write_wrong_args(out, "expireat");
    };
    let unix = parse_int!(out, ts.as_str(), u64);
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let diff = if unix > now_unix {
        Duration::from_secs(unix - now_unix)
    } else {
        Duration::ZERO
    };
    resp::write_boolean(out, store.expireat(key, Instant::now() + diff));
}

pub fn persist(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key] => resp::write_boolean(out, store.persist(key)),
        _ => resp::write_wrong_args(out, "persist"),
    }
}

pub fn rename(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    let [_, old, new] = parts else {
        return resp::write_wrong_args(out, "rename");
    };
    if store.rename(old, new) {
        resp::write_ok(out)
    } else {
        resp::write_err(out, "no such key")
    }
}

pub fn renamenx(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, old, new] => resp::write_boolean(out, store.renamenx(old, new)),
        _ => resp::write_wrong_args(out, "renamenx"),
    }
}

pub fn copy(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    let replace = parts.iter().any(|p| p.eq_ignore_ascii_case("REPLACE"));
    match parts {
        [_, src, dst, ..] => resp::write_boolean(out, store.copy(src, dst, replace)),
        _ => resp::write_wrong_args(out, "copy"),
    }
}

pub fn randomkey(_parts: &[String], store: &Store, out: &mut Vec<u8>) {
    resp::write_opt_bulk(out, store.randomkey());
}

pub fn keys(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    let [_, pattern] = parts else {
        return resp::write_wrong_args(out, "keys");
    };
    let mut all = store.keys_matching(pattern);
    all.sort();
    resp::write_array(out, &all);
}
