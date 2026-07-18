use crate::storage::store::Store;
use crate::utils::resp;
use crate::utils::util::format_float;
use crate::{parse_float, parse_int, store_ok, wt};

pub fn hset(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key, pairs @ ..] if !pairs.is_empty() && pairs.len() % 2 == 0 => {
            let fields = pairs
                .chunks(2)
                .map(|c| (c[0].to_string(), c[1].to_string()))
                .collect();
            resp::write_integer(out, wt!(out, store.hset(key, fields)) as i64);
        }
        _ => resp::write_wrong_args(out, "hset"),
    }
}

pub fn hsetnx(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key, field, value] => {
            resp::write_boolean(out, wt!(out, store.hsetnx(key, field, value.to_string())))
        }
        _ => resp::write_wrong_args(out, "hsetnx"),
    }
}

pub fn hget(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key, field] => resp::write_opt_bulk(out, wt!(out, store.hget(key, field))),
        _ => resp::write_wrong_args(out, "hget"),
    }
}

pub fn hmget(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key, fields @ ..] if !fields.is_empty() => {
            resp::write_opt_array(out, &wt!(out, store.hmget(key, fields)))
        }
        _ => resp::write_wrong_args(out, "hmget"),
    }
}

pub fn hmset(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key, pairs @ ..] if !pairs.is_empty() && pairs.len() % 2 == 0 => {
            let fields = pairs
                .chunks(2)
                .map(|c| (c[0].to_string(), c[1].to_string()))
                .collect();
            wt!(out, store.hset(key, fields));
            resp::write_ok(out);
        }
        _ => resp::write_wrong_args(out, "hmset"),
    }
}

pub fn hgetall(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let [_, key] = parts else {
        return resp::write_wrong_args(out, "hgetall");
    };
    let pairs = wt!(out, store.hgetall(key));
    resp::write_array_header(out, pairs.len() * 2);
    for (f, v) in pairs {
        resp::write_bulk(out, &f);
        resp::write_bulk(out, &v);
    }
}

pub fn hdel(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key, fields @ ..] if !fields.is_empty() => {
            resp::write_integer(out, wt!(out, store.hdel(key, fields)) as i64)
        }
        _ => resp::write_wrong_args(out, "hdel"),
    }
}

pub fn hexists(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key, field] => resp::write_boolean(out, wt!(out, store.hexists(key, field))),
        _ => resp::write_wrong_args(out, "hexists"),
    }
}

pub fn hlen(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key] => resp::write_integer(out, wt!(out, store.hlen(key)) as i64),
        _ => resp::write_wrong_args(out, "hlen"),
    }
}

pub fn hkeys(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key] => resp::write_array(out, &wt!(out, store.hkeys(key))),
        _ => resp::write_wrong_args(out, "hkeys"),
    }
}

pub fn hvals(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key] => resp::write_array(out, &wt!(out, store.hvals(key))),
        _ => resp::write_wrong_args(out, "hvals"),
    }
}

pub fn hincrby(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let [_, key, field, by] = parts else {
        return resp::write_wrong_args(out, "hincrby");
    };
    let n = parse_int!(out, by);
    resp::write_integer(out, store_ok!(out, store.hincrby(key, field, n)));
}

pub fn hincrbyfloat(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let [_, key, field, by] = parts else {
        return resp::write_wrong_args(out, "hincrbyfloat");
    };
    let n = parse_float!(out, by);
    resp::write_bulk(
        out,
        &format_float(store_ok!(out, store.hincrbyfloat(key, field, n))),
    );
}
