use crate::storage::{
    store::Store,
    value::{StoreValue, now_ms},
};
use crate::utils::resp;
use crate::utils::util::format_float;
use crate::{parse_float, parse_int, store_ok, wt};

pub fn set(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let [_, key, value, rest @ ..] = parts else {
        return resp::write_wrong_args(out, "set");
    };

    let mut expires_ms: u64 = 0;
    let mut nx = false;
    let mut xx = false;
    let mut get = false;

    let mut i = 0;
    while i < rest.len() {
        let opt = rest[i].as_bytes();
        if opt.eq_ignore_ascii_case(b"EX") {
            i += 1;
            let s = parse_int!(out, rest.get(i).map(|s| *s).unwrap_or(""), u64);
            expires_ms = now_ms() + s * 1000;
        } else if opt.eq_ignore_ascii_case(b"PX") {
            i += 1;
            let ms = parse_int!(out, rest.get(i).map(|s| *s).unwrap_or(""), u64);
            expires_ms = now_ms() + ms;
        } else if opt.eq_ignore_ascii_case(b"NX") {
            nx = true;
        } else if opt.eq_ignore_ascii_case(b"XX") {
            xx = true;
        } else if opt.eq_ignore_ascii_case(b"GET") {
            get = true;
        } else {
            resp::write_err(out, "syntax error");
            return;
        }
        i += 1;
    }

    if nx || xx || get {
        let existing = store.get(key);
        let key_exists = existing.is_some();

        if nx && key_exists {
            return resp::write_nil(out);
        }
        if xx && !key_exists {
            return resp::write_nil(out);
        }

        let new_val = StoreValue {
            value: crate::storage::value::FlashDB::String(value.to_string()),
            expires_ms,
        };
        store.data.set(key, new_val, || key.to_string());

        if get {
            resp::write_opt_bulk(out, existing);
        } else {
            resp::write_ok(out);
        }
        return;
    }

    store.set_string(key, value, expires_ms);
    resp::write_ok(out);
}

pub fn setnx(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key, value] => {
            resp::write_boolean(out, store.setnx(key.to_string(), value.to_string()))
        }
        _ => resp::write_wrong_args(out, "setnx"),
    }
}

pub fn setex(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let [_, key, secs, value] = parts else {
        return resp::write_wrong_args(out, "setex");
    };
    let s = parse_int!(out, secs, u64);
    store.set(
        key.to_string(),
        StoreValue {
            value: crate::storage::value::FlashDB::String(value.to_string()),
            expires_ms: now_ms() + s * 1000,
        },
    );
    resp::write_ok(out);
}

pub fn psetex(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let [_, key, ms, value] = parts else {
        return resp::write_wrong_args(out, "psetex");
    };
    let m = parse_int!(out, ms, u64);
    store.set(
        key.to_string(),
        StoreValue {
            value: crate::storage::value::FlashDB::String(value.to_string()),
            expires_ms: now_ms() + m,
        },
    );
    resp::write_ok(out);
}

pub fn get(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key] => {
            if !store.get_to_buf(key, out) {
                resp::write_nil(out);
            }
        }
        _ => resp::write_wrong_args(out, "get"),
    }
}

pub fn getdel(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key] => resp::write_opt_bulk(out, store.getdel(key)),
        _ => resp::write_wrong_args(out, "getdel"),
    }
}

pub fn getset(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key, value] => {
            resp::write_opt_bulk(out, store.getset(key.to_string(), value.to_string()))
        }
        _ => resp::write_wrong_args(out, "getset"),
    }
}

pub fn getex(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let [_, key, rest @ ..] = parts else {
        return resp::write_wrong_args(out, "getex");
    };
    let expires_ms: u64 = match rest {
        [] => match store.ttl(key) {
            Some(d) => now_ms() + d.as_millis() as u64,
            None => 0,
        },
        [opt] if opt.eq_ignore_ascii_case("PERSIST") => 0,
        [opt, secs] if opt.eq_ignore_ascii_case("EX") => {
            let s = parse_int!(out, secs, u64);
            now_ms() + s * 1000
        }
        [opt, ms] if opt.eq_ignore_ascii_case("PX") => {
            let m = parse_int!(out, ms, u64);
            now_ms() + m
        }
        _ => {
            resp::write_err(out, "syntax error");
            return;
        }
    };
    resp::write_opt_bulk(out, store.getex_ms(key, expires_ms));
}

pub fn mset(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, items @ ..] if !items.is_empty() && items.len() % 2 == 0 => {
            for chunk in items.chunks(2) {
                store.set(
                    chunk[0].to_string(),
                    StoreValue::string(chunk[1].to_string()),
                );
            }
            resp::write_ok(out);
        }
        _ => resp::write_wrong_args(out, "mset"),
    }
}

pub fn msetnx(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, items @ ..] if !items.is_empty() && items.len() % 2 == 0 => {
            if items.chunks(2).any(|c| store.exists(&c[0])) {
                return out.extend_from_slice(resp::ZERO);
            }
            for chunk in items.chunks(2) {
                store.set(
                    chunk[0].to_string(),
                    StoreValue::string(chunk[1].to_string()),
                );
            }
            out.extend_from_slice(resp::ONE);
        }
        _ => resp::write_wrong_args(out, "msetnx"),
    }
}

pub fn mget(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, keys @ ..] if !keys.is_empty() => {
            let vals: Vec<Option<String>> = keys.iter().map(|k| store.get(k)).collect();
            resp::write_opt_array(out, &vals);
        }
        _ => resp::write_wrong_args(out, "mget"),
    }
}

pub fn incr(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key] => resp::write_integer(out, store_ok!(out, store.incr(key))),
        _ => resp::write_wrong_args(out, "incr"),
    }
}

pub fn decr(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key] => resp::write_integer(out, store_ok!(out, store.decr(key))),
        _ => resp::write_wrong_args(out, "decr"),
    }
}

pub fn incrby(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let [_, key, by] = parts else {
        return resp::write_wrong_args(out, "incrby");
    };
    let n = parse_int!(out, by);
    resp::write_integer(out, store_ok!(out, store.incrby(key, n)));
}

pub fn decrby(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let [_, key, by] = parts else {
        return resp::write_wrong_args(out, "decrby");
    };
    let n = parse_int!(out, by);
    resp::write_integer(out, store_ok!(out, store.decrby(key, n)));
}

pub fn incrbyfloat(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let [_, key, by] = parts else {
        return resp::write_wrong_args(out, "incrbyfloat");
    };
    let n = parse_float!(out, by);
    resp::write_bulk(
        out,
        &format_float(store_ok!(out, store.incrbyfloat(key, n))),
    );
}

pub fn append(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key, value] => resp::write_integer(out, wt!(out, store.append(key, value)) as i64),
        _ => resp::write_wrong_args(out, "append"),
    }
}

pub fn strlen(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key] => resp::write_integer(out, wt!(out, store.strlen(key)) as i64),
        _ => resp::write_wrong_args(out, "strlen"),
    }
}

pub fn getrange(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let [_, key, start, end] = parts else {
        return resp::write_wrong_args(out, "getrange");
    };
    let s = parse_int!(out, start);
    let e = parse_int!(out, end);
    resp::write_bulk(out, &store.getrange(key, s, e));
}

pub fn setrange(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let [_, key, offset, value] = parts else {
        return resp::write_wrong_args(out, "setrange");
    };
    let o = parse_int!(out, offset, usize);
    resp::write_integer(out, wt!(out, store.setrange(key, o, value)) as i64);
}
