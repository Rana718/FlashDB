use crate::storage::{store::Store, value::StoreValue};
use crate::utils::resp::{self, NIL, OK, ONE, ZERO};
use crate::utils::util::format_float;
use std::time::Duration;
use std::time::Instant;

pub fn set(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key, value, rest @ ..] => {
            let mut expires_at = None;
            let mut nx = false;
            let mut xx = false;
            let mut get = false;

            let mut i = 0;
            while i < rest.len() {
                match rest[i].to_ascii_uppercase().as_str() {
                    "EX" => {
                        i += 1;
                        match rest.get(i).and_then(|s| s.parse::<u64>().ok()) {
                            Some(s) => expires_at = Some(Instant::now() + Duration::from_secs(s)),
                            None => return resp::err("invalid expire time"),
                        }
                    }
                    "PX" => {
                        i += 1;
                        match rest.get(i).and_then(|s| s.parse::<u64>().ok()) {
                            Some(ms) => {
                                expires_at = Some(Instant::now() + Duration::from_millis(ms))
                            }
                            None => return resp::err("invalid expire time"),
                        }
                    }
                    "NX" => nx = true,
                    "XX" => xx = true,
                    "GET" => get = true,
                    _ => return resp::err("syntax error"),
                }
                i += 1;
            }

            let key_exists = store.exists(key);
            if nx && key_exists {
                return NIL.into();
            }
            if xx && !key_exists {
                return NIL.into();
            }

            let old = if get { store.get(key) } else { None };
            store.set(
                key.clone(),
                StoreValue::string_with_expiry(value.clone(), expires_at),
            );

            if get { resp::opt_bulk(old) } else { OK.into() }
        }
        _ => resp::wrong_args("set"),
    }
}

pub fn setnx(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key, value] => resp::boolean(store.setnx(key.clone(), value.clone())),
        _ => resp::wrong_args("setnx"),
    }
}

pub fn setex(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key, secs, value] => match secs.parse::<u64>() {
            Ok(s) => {
                store.set(
                    key.clone(),
                    StoreValue::string_with_expiry(
                        value.clone(),
                        Some(Instant::now() + Duration::from_secs(s)),
                    ),
                );
                OK.into()
            }
            Err(_) => resp::err("invalid expire time"),
        },
        _ => resp::wrong_args("setex"),
    }
}

pub fn psetex(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key, ms, value] => match ms.parse::<u64>() {
            Ok(m) => {
                store.set(
                    key.clone(),
                    StoreValue::string_with_expiry(
                        value.clone(),
                        Some(Instant::now() + Duration::from_millis(m)),
                    ),
                );
                OK.into()
            }
            Err(_) => resp::err("invalid expire time"),
        },
        _ => resp::wrong_args("psetex"),
    }
}

pub fn get(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key] => resp::opt_bulk(store.get(key)),
        _ => resp::wrong_args("get"),
    }
}

pub fn getdel(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key] => resp::opt_bulk(store.getdel(key)),
        _ => resp::wrong_args("getdel"),
    }
}

pub fn getset(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key, value] => resp::opt_bulk(store.getset(key.clone(), value.clone())),
        _ => resp::wrong_args("getset"),
    }
}

pub fn getex(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key, rest @ ..] => {
            let expires_at = match rest {
                [] => store.ttl(key).map(|d| Instant::now() + d),
                [opt] if opt.eq_ignore_ascii_case("PERSIST") => None,
                [opt, secs] if opt.eq_ignore_ascii_case("EX") => match secs.parse::<u64>() {
                    Ok(s) => Some(Instant::now() + Duration::from_secs(s)),
                    Err(_) => return resp::err("invalid expire time"),
                },
                [opt, ms] if opt.eq_ignore_ascii_case("PX") => match ms.parse::<u64>() {
                    Ok(m) => Some(Instant::now() + Duration::from_millis(m)),
                    Err(_) => return resp::err("invalid expire time"),
                },
                _ => return resp::err("syntax error"),
            };
            resp::opt_bulk(store.getex(key, expires_at))
        }
        _ => resp::wrong_args("getex"),
    }
}

pub fn mset(parts: &[String], store: &Store) -> String {
    match parts {
        [_, items @ ..] if !items.is_empty() && items.len() % 2 == 0 => {
            for chunk in items.chunks(2) {
                store.set(chunk[0].clone(), StoreValue::string(chunk[1].clone()));
            }
            OK.into()
        }
        _ => resp::wrong_args("mset"),
    }
}

pub fn msetnx(parts: &[String], store: &Store) -> String {
    match parts {
        [_, items @ ..] if !items.is_empty() && items.len() % 2 == 0 => {
            if items.chunks(2).any(|c| store.exists(&c[0])) {
                return ZERO.into();
            }
            for chunk in items.chunks(2) {
                store.set(chunk[0].clone(), StoreValue::string(chunk[1].clone()));
            }
            ONE.into()
        }
        _ => resp::wrong_args("msetnx"),
    }
}

pub fn mget(parts: &[String], store: &Store) -> String {
    match parts {
        [_, keys @ ..] if !keys.is_empty() => {
            resp::opt_array(&keys.iter().map(|k| store.get(k)).collect::<Vec<_>>())
        }
        _ => resp::wrong_args("mget"),
    }
}

pub fn incr(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key] => match store.incr(key) {
            Ok(n) => resp::integer(n),
            Err(e) => resp::err(e),
        },
        _ => resp::wrong_args("incr"),
    }
}

pub fn decr(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key] => match store.decr(key) {
            Ok(n) => resp::integer(n),
            Err(e) => resp::err(e),
        },
        _ => resp::wrong_args("decr"),
    }
}

pub fn incrby(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key, by] => match by.parse::<i64>() {
            Ok(n) => match store.incrby(key, n) {
                Ok(v) => resp::integer(v),
                Err(e) => resp::err(e),
            },
            Err(_) => resp::err("value is not an integer"),
        },
        _ => resp::wrong_args("incrby"),
    }
}

pub fn decrby(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key, by] => match by.parse::<i64>() {
            Ok(n) => match store.decrby(key, n) {
                Ok(v) => resp::integer(v),
                Err(e) => resp::err(e),
            },
            Err(_) => resp::err("value is not an integer"),
        },
        _ => resp::wrong_args("decrby"),
    }
}

pub fn incrbyfloat(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key, by] => match by.parse::<f64>() {
            Ok(n) => match store.incrbyfloat(key, n) {
                Ok(v) => resp::bulk(&format_float(v)),
                Err(e) => resp::err(e),
            },
            Err(_) => resp::err("value is not a float"),
        },
        _ => resp::wrong_args("incrbyfloat"),
    }
}

pub fn append(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key, value] => match store.append(key, value) {
            Ok(len) => resp::integer(len as i64),
            Err(_) => resp::wrong_type(),
        },
        _ => resp::wrong_args("append"),
    }
}

pub fn strlen(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key] => match store.strlen(key) {
            Ok(len) => resp::integer(len as i64),
            Err(_) => resp::wrong_type(),
        },
        _ => resp::wrong_args("strlen"),
    }
}

pub fn getrange(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key, start, end] => {
            let (Ok(s), Ok(e)) = (start.parse::<i64>(), end.parse::<i64>()) else {
                return resp::err("value is not an integer");
            };
            resp::bulk(&store.getrange(key, s, e))
        }
        _ => resp::wrong_args("getrange"),
    }
}

pub fn setrange(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key, offset, value] => match offset.parse::<usize>() {
            Ok(o) => match store.setrange(key, o, value) {
                Ok(len) => resp::integer(len as i64),
                Err(_) => resp::wrong_type(),
            },
            Err(_) => resp::err("value is not an integer"),
        },
        _ => resp::wrong_args("setrange"),
    }
}
