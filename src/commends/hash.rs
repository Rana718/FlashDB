use crate::utils::resp::{self, OK};
use crate::storage::store::Store;
use crate::utils::util::format_float;

pub async fn hset(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key, pairs @ ..] if !pairs.is_empty() && pairs.len() % 2 == 0 => {
            let fields = pairs.chunks(2).map(|c| (c[0].clone(), c[1].clone())).collect();
            match store.hset(key, fields) {
                Ok(n) => resp::integer(n as i64),
                Err(_) => resp::wrong_type(),
            }
        }
        _ => resp::wrong_args("hset"),
    }
}

pub async fn hsetnx(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key, field, value] => match store.hsetnx(key, field, value.clone()) {
            Ok(set) => resp::boolean(set),
            Err(_) => resp::wrong_type(),
        },
        _ => resp::wrong_args("hsetnx"),
    }
}

pub async fn hget(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key, field] => match store.hget(key, field) {
            Ok(v) => resp::opt_bulk(v),
            Err(_) => resp::wrong_type(),
        },
        _ => resp::wrong_args("hget"),
    }
}

pub async fn hmget(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key, fields @ ..] if !fields.is_empty() => match store.hmget(key, fields) {
            Ok(vals) => resp::opt_array(&vals),
            Err(_) => resp::wrong_type(),
        },
        _ => resp::wrong_args("hmget"),
    }
}

pub async fn hmset(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key, pairs @ ..] if !pairs.is_empty() && pairs.len() % 2 == 0 => {
            let fields = pairs.chunks(2).map(|c| (c[0].clone(), c[1].clone())).collect();
            match store.hset(key, fields) {
                Ok(_) => OK.into(),
                Err(_) => resp::wrong_type(),
            }
        }
        _ => resp::wrong_args("hmset"),
    }
}

pub async fn hgetall(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key] => match store.hgetall(key) {
            Ok(pairs) => {
                let mut out = format!("*{}\r\n", pairs.len() * 2);
                for (f, v) in pairs {
                    out.push_str(&resp::bulk(&f));
                    out.push_str(&resp::bulk(&v));
                }
                out
            }
            Err(_) => resp::wrong_type(),
        },
        _ => resp::wrong_args("hgetall"),
    }
}

pub async fn hdel(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key, fields @ ..] if !fields.is_empty() => match store.hdel(key, fields) {
            Ok(n) => resp::integer(n as i64),
            Err(_) => resp::wrong_type(),
        },
        _ => resp::wrong_args("hdel"),
    }
}

pub async fn hexists(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key, field] => match store.hexists(key, field) {
            Ok(b) => resp::boolean(b),
            Err(_) => resp::wrong_type(),
        },
        _ => resp::wrong_args("hexists"),
    }
}

pub async fn hlen(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key] => match store.hlen(key) {
            Ok(n) => resp::integer(n as i64),
            Err(_) => resp::wrong_type(),
        },
        _ => resp::wrong_args("hlen"),
    }
}

pub async fn hkeys(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key] => match store.hkeys(key) {
            Ok(keys) => resp::array(&keys),
            Err(_) => resp::wrong_type(),
        },
        _ => resp::wrong_args("hkeys"),
    }
}

pub async fn hvals(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key] => match store.hvals(key) {
            Ok(vals) => resp::array(&vals),
            Err(_) => resp::wrong_type(),
        },
        _ => resp::wrong_args("hvals"),
    }
}

pub async fn hincrby(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key, field, by] => match by.parse::<i64>() {
            Ok(n) => match store.hincrby(key, field, n) {
                Ok(v) => resp::integer(v),
                Err(e) => resp::err(e),
            },
            Err(_) => resp::err("value is not an integer"),
        },
        _ => resp::wrong_args("hincrby"),
    }
}

pub async fn hincrbyfloat(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key, field, by] => match by.parse::<f64>() {
            Ok(n) => match store.hincrbyfloat(key, field, n) {
                Ok(v) => resp::bulk(&format_float(v)),
                Err(e) => resp::err(e),
            },
            Err(_) => resp::err("value is not a float"),
        },
        _ => resp::wrong_args("hincrbyfloat"),
    }
}
