use crate::storage::{store::Store, value::StoreValue};
use crate::utils::resp;
use crate::utils::util::format_float;
use crate::{parse_float, parse_int, store_ok, wt};
use std::time::{Duration, Instant};

pub fn set(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    let [_, key, value, rest @ ..] = parts else {
        return resp::write_wrong_args(out, "set");
    };

    let mut expires_at = None;
    let mut nx = false;
    let mut xx = false;
    let mut get = false;

    let mut i = 0;
    while i < rest.len() {
        match rest[i].to_ascii_uppercase().as_str() {
            "EX" => {
                i += 1;
                let s = parse_int!(out, rest.get(i).map(|s| s.as_str()).unwrap_or(""), u64);
                expires_at = Some(Instant::now() + Duration::from_secs(s));
            }
            "PX" => {
                i += 1;
                let ms = parse_int!(out, rest.get(i).map(|s| s.as_str()).unwrap_or(""), u64);
                expires_at = Some(Instant::now() + Duration::from_millis(ms));
            }
            "NX" => nx = true,
            "XX" => xx = true,
            "GET" => get = true,
            _ => {
                resp::write_err(out, "syntax error");
                return;
            }
        }
        i += 1;
    }

    let key_exists = store.exists(key);
    if nx && key_exists {
        return resp::write_nil(out);
    }
    if xx && !key_exists {
        return resp::write_nil(out);
    }

    let old = if get { store.get(key) } else { None };
    store.set(
        key.clone(),
        StoreValue::string_with_expiry(value.clone(), expires_at),
    );
    if get {
        resp::write_opt_bulk(out, old)
    } else {
        resp::write_ok(out)
    }
}

pub fn setnx(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key, value] => resp::write_boolean(out, store.setnx(key.clone(), value.clone())),
        _ => resp::write_wrong_args(out, "setnx"),
    }
}

pub fn setex(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    let [_, key, secs, value] = parts else {
        return resp::write_wrong_args(out, "setex");
    };
    let s = parse_int!(out, secs.as_str(), u64);
    store.set(
        key.clone(),
        StoreValue::string_with_expiry(
            value.clone(),
            Some(Instant::now() + Duration::from_secs(s)),
        ),
    );
    resp::write_ok(out);
}

pub fn psetex(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    let [_, key, ms, value] = parts else {
        return resp::write_wrong_args(out, "psetex");
    };
    let m = parse_int!(out, ms.as_str(), u64);
    store.set(
        key.clone(),
        StoreValue::string_with_expiry(
            value.clone(),
            Some(Instant::now() + Duration::from_millis(m)),
        ),
    );
    resp::write_ok(out);
}

pub fn get(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key] => resp::write_opt_bulk(out, store.get(key)),
        _ => resp::write_wrong_args(out, "get"),
    }
}

pub fn getdel(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key] => resp::write_opt_bulk(out, store.getdel(key)),
        _ => resp::write_wrong_args(out, "getdel"),
    }
}

pub fn getset(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key, value] => resp::write_opt_bulk(out, store.getset(key.clone(), value.clone())),
        _ => resp::write_wrong_args(out, "getset"),
    }
}

pub fn getex(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    let [_, key, rest @ ..] = parts else {
        return resp::write_wrong_args(out, "getex");
    };
    let expires_at = match rest {
        [] => store.ttl(key).map(|d| Instant::now() + d),
        [opt] if opt.eq_ignore_ascii_case("PERSIST") => None,
        [opt, secs] if opt.eq_ignore_ascii_case("EX") => {
            let s = parse_int!(out, secs.as_str(), u64);
            Some(Instant::now() + Duration::from_secs(s))
        }
        [opt, ms] if opt.eq_ignore_ascii_case("PX") => {
            let m = parse_int!(out, ms.as_str(), u64);
            Some(Instant::now() + Duration::from_millis(m))
        }
        _ => {
            resp::write_err(out, "syntax error");
            return;
        }
    };
    resp::write_opt_bulk(out, store.getex(key, expires_at));
}

pub fn mset(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, items @ ..] if !items.is_empty() && items.len() % 2 == 0 => {
            for chunk in items.chunks(2) {
                store.set(chunk[0].clone(), StoreValue::string(chunk[1].clone()));
            }
            resp::write_ok(out);
        }
        _ => resp::write_wrong_args(out, "mset"),
    }
}

pub fn msetnx(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, items @ ..] if !items.is_empty() && items.len() % 2 == 0 => {
            if items.chunks(2).any(|c| store.exists(&c[0])) {
                return out.extend_from_slice(resp::ZERO);
            }
            for chunk in items.chunks(2) {
                store.set(chunk[0].clone(), StoreValue::string(chunk[1].clone()));
            }
            out.extend_from_slice(resp::ONE);
        }
        _ => resp::write_wrong_args(out, "msetnx"),
    }
}

pub fn mget(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, keys @ ..] if !keys.is_empty() => {
            let vals: Vec<Option<String>> = keys.iter().map(|k| store.get(k)).collect();
            resp::write_opt_array(out, &vals);
        }
        _ => resp::write_wrong_args(out, "mget"),
    }
}

pub fn incr(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key] => resp::write_integer(out, store_ok!(out, store.incr(key))),
        _ => resp::write_wrong_args(out, "incr"),
    }
}

pub fn decr(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key] => resp::write_integer(out, store_ok!(out, store.decr(key))),
        _ => resp::write_wrong_args(out, "decr"),
    }
}

pub fn incrby(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    let [_, key, by] = parts else {
        return resp::write_wrong_args(out, "incrby");
    };
    let n = parse_int!(out, by.as_str());
    resp::write_integer(out, store_ok!(out, store.incrby(key, n)));
}

pub fn decrby(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    let [_, key, by] = parts else {
        return resp::write_wrong_args(out, "decrby");
    };
    let n = parse_int!(out, by.as_str());
    resp::write_integer(out, store_ok!(out, store.decrby(key, n)));
}

pub fn incrbyfloat(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    let [_, key, by] = parts else {
        return resp::write_wrong_args(out, "incrbyfloat");
    };
    let n = parse_float!(out, by.as_str());
    resp::write_bulk(
        out,
        &format_float(store_ok!(out, store.incrbyfloat(key, n))),
    );
}

pub fn append(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key, value] => resp::write_integer(out, wt!(out, store.append(key, value)) as i64),
        _ => resp::write_wrong_args(out, "append"),
    }
}

pub fn strlen(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key] => resp::write_integer(out, wt!(out, store.strlen(key)) as i64),
        _ => resp::write_wrong_args(out, "strlen"),
    }
}

pub fn getrange(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    let [_, key, start, end] = parts else {
        return resp::write_wrong_args(out, "getrange");
    };
    let s = parse_int!(out, start.as_str());
    let e = parse_int!(out, end.as_str());
    resp::write_bulk(out, &store.getrange(key, s, e));
}

pub fn setrange(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    let [_, key, offset, value] = parts else {
        return resp::write_wrong_args(out, "setrange");
    };
    let o = parse_int!(out, offset.as_str(), usize);
    resp::write_integer(out, wt!(out, store.setrange(key, o, value)) as i64);
}
