use crate::storage::store::Store;
use crate::utils::resp::{self, OK};
use crate::utils::util::format_float;

pub fn hset(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key, pairs @ ..] if !pairs.is_empty() && pairs.len() % 2 == 0 => {
            let fields = pairs
                .chunks(2)
                .map(|c| (c[0].clone(), c[1].clone()))
                .collect();
            match store.hset(key, fields) {
                Ok(n) => resp::integer(n as i64),
                Err(_) => resp::wrong_type(),
            }
        }
        _ => resp::wrong_args("hset"),
    }
}

pub fn hsetnx(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key, field, value] => match store.hsetnx(key, field, value.clone()) {
            Ok(set) => resp::boolean(set),
            Err(_) => resp::wrong_type(),
        },
        _ => resp::wrong_args("hsetnx"),
    }
}

pub fn hget(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key, field] => match store.hget(key, field) {
            Ok(v) => resp::opt_bulk(v),
            Err(_) => resp::wrong_type(),
        },
        _ => resp::wrong_args("hget"),
    }
}

pub fn hmget(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key, fields @ ..] if !fields.is_empty() => match store.hmget(key, fields) {
            Ok(vals) => resp::opt_array(&vals),
            Err(_) => resp::wrong_type(),
        },
        _ => resp::wrong_args("hmget"),
    }
}

pub fn hmset(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key, pairs @ ..] if !pairs.is_empty() && pairs.len() % 2 == 0 => {
            let fields = pairs
                .chunks(2)
                .map(|c| (c[0].clone(), c[1].clone()))
                .collect();
            match store.hset(key, fields) {
                Ok(_) => OK.into(),
                Err(_) => resp::wrong_type(),
            }
        }
        _ => resp::wrong_args("hmset"),
    }
}

pub fn hgetall(parts: &[String], store: &Store) -> String {
    match parts {
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

pub fn hdel(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key, fields @ ..] if !fields.is_empty() => match store.hdel(key, fields) {
            Ok(n) => resp::integer(n as i64),
            Err(_) => resp::wrong_type(),
        },
        _ => resp::wrong_args("hdel"),
    }
}

pub fn hexists(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key, field] => match store.hexists(key, field) {
            Ok(b) => resp::boolean(b),
            Err(_) => resp::wrong_type(),
        },
        _ => resp::wrong_args("hexists"),
    }
}

pub fn hlen(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key] => match store.hlen(key) {
            Ok(n) => resp::integer(n as i64),
            Err(_) => resp::wrong_type(),
        },
        _ => resp::wrong_args("hlen"),
    }
}

pub fn hkeys(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key] => match store.hkeys(key) {
            Ok(keys) => resp::array(&keys),
            Err(_) => resp::wrong_type(),
        },
        _ => resp::wrong_args("hkeys"),
    }
}

pub fn hvals(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key] => match store.hvals(key) {
            Ok(vals) => resp::array(&vals),
            Err(_) => resp::wrong_type(),
        },
        _ => resp::wrong_args("hvals"),
    }
}

pub fn hincrby(parts: &[String], store: &Store) -> String {
    match parts {
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

pub fn hincrbyfloat(parts: &[String], store: &Store) -> String {
    match parts {
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
