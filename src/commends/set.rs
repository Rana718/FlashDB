use crate::storage::store::Store;
use crate::utils::resp;
use crate::{parse_int, store_ok, wt};

pub fn sadd(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key, members @ ..] if !members.is_empty() => {
            resp::write_integer(out, wt!(out, store.sadd(key, members)) as i64)
        }
        _ => resp::write_wrong_args(out, "sadd"),
    }
}

pub fn srem(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key, members @ ..] if !members.is_empty() => {
            resp::write_integer(out, wt!(out, store.srem(key, members)) as i64)
        }
        _ => resp::write_wrong_args(out, "srem"),
    }
}

pub fn sismember(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key, member] => resp::write_boolean(out, wt!(out, store.sismember(key, member))),
        _ => resp::write_wrong_args(out, "sismember"),
    }
}

pub fn smismember(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key, members @ ..] if !members.is_empty() => {
            let results = wt!(out, store.smismember(key, members));
            resp::write_array_header(out, results.len());
            for r in results {
                resp::write_integer(out, if r { 1 } else { 0 });
            }
        }
        _ => resp::write_wrong_args(out, "smismember"),
    }
}

pub fn smembers(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key] => resp::write_array(out, &wt!(out, store.smembers(key))),
        _ => resp::write_wrong_args(out, "smembers"),
    }
}

pub fn scard(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key] => resp::write_integer(out, wt!(out, store.scard(key)) as i64),
        _ => resp::write_wrong_args(out, "scard"),
    }
}

pub fn spop(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let (key, count) = match parts {
        [_, key] => (*key, 1usize),
        [_, key, cnt] => {
            let c = parse_int!(out, cnt, usize);
            (*key, c)
        }
        _ => return resp::write_wrong_args(out, "spop"),
    };
    let items = wt!(out, store.spop(key, count));
    if parts.len() == 2 {
        if items.is_empty() {
            resp::write_nil(out);
        } else {
            resp::write_bulk(out, &items[0]);
        }
    } else {
        resp::write_array(out, &items);
    }
}

pub fn srandmember(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let (key, count) = match parts {
        [_, key] => (*key, 1i64),
        [_, key, cnt] => {
            let c = parse_int!(out, cnt);
            (*key, c)
        }
        _ => return resp::write_wrong_args(out, "srandmember"),
    };
    let items = wt!(out, store.srandmember(key, count));
    if parts.len() == 2 {
        if items.is_empty() {
            resp::write_nil(out);
        } else {
            resp::write_bulk(out, &items[0]);
        }
    } else {
        resp::write_array(out, &items);
    }
}

pub fn smove(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, src, dst, member] => resp::write_boolean(out, wt!(out, store.smove(src, dst, member))),
        _ => resp::write_wrong_args(out, "smove"),
    }
}

pub fn sunion(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, keys @ ..] if !keys.is_empty() => {
            resp::write_array(out, &wt!(out, store.sunion(keys)))
        }
        _ => resp::write_wrong_args(out, "sunion"),
    }
}

pub fn sinter(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, keys @ ..] if !keys.is_empty() => {
            resp::write_array(out, &wt!(out, store.sinter(keys)))
        }
        _ => resp::write_wrong_args(out, "sinter"),
    }
}

pub fn sdiff(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, keys @ ..] if !keys.is_empty() => {
            resp::write_array(out, &wt!(out, store.sdiff(keys)))
        }
        _ => resp::write_wrong_args(out, "sdiff"),
    }
}

pub fn sunionstore(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, dst, keys @ ..] if !keys.is_empty() => {
            resp::write_integer(out, store_ok!(out, store.sunionstore(dst, keys)) as i64)
        }
        _ => resp::write_wrong_args(out, "sunionstore"),
    }
}

pub fn sinterstore(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, dst, keys @ ..] if !keys.is_empty() => {
            resp::write_integer(out, store_ok!(out, store.sinterstore(dst, keys)) as i64)
        }
        _ => resp::write_wrong_args(out, "sinterstore"),
    }
}

pub fn sdiffstore(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, dst, keys @ ..] if !keys.is_empty() => {
            resp::write_integer(out, store_ok!(out, store.sdiffstore(dst, keys)) as i64)
        }
        _ => resp::write_wrong_args(out, "sdiffstore"),
    }
}

pub fn sintercard(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let [_, numkeys_str, rest @ ..] = parts else {
        return resp::write_wrong_args(out, "sintercard");
    };
    let numkeys = parse_int!(out, numkeys_str, usize);
    if rest.len() < numkeys {
        return resp::write_wrong_args(out, "sintercard");
    }
    let keys = &rest[..numkeys];
    let mut limit = 0usize;
    let remaining = &rest[numkeys..];
    let mut i = 0;
    while i < remaining.len() {
        if remaining[i].eq_ignore_ascii_case("LIMIT") {
            i += 1;
            if i >= remaining.len() {
                return resp::write_err(out, "syntax error");
            }
            limit = parse_int!(out, remaining[i], usize);
        } else {
            return resp::write_err(out, "syntax error");
        }
        i += 1;
    }
    resp::write_integer(out, store_ok!(out, store.sintercard(keys, limit)) as i64);
}
