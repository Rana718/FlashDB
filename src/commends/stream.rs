use crate::storage::store::Store;
use crate::utils::resp;
use crate::{parse_int, store_ok, wt};

pub fn xadd(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let [_, key, rest @ ..] = parts else {
        return resp::write_wrong_args(out, "xadd");
    };
    if rest.is_empty() {
        return resp::write_wrong_args(out, "xadd");
    }

    let mut nomkstream = false;
    let mut maxlen: Option<usize> = None;
    let mut i = 0;

    loop {
        if i >= rest.len() {
            return resp::write_wrong_args(out, "xadd");
        }
        if rest[i].eq_ignore_ascii_case("NOMKSTREAM") {
            nomkstream = true;
            i += 1;
        } else if rest[i].eq_ignore_ascii_case("MAXLEN") {
            i += 1;
            if i >= rest.len() {
                return resp::write_err(out, "syntax error");
            }
            if rest[i] == "~" || rest[i] == "=" {
                i += 1;
            }
            if i >= rest.len() {
                return resp::write_err(out, "syntax error");
            }
            maxlen = Some(parse_int!(out, rest[i], usize));
            i += 1;
        } else if rest[i].eq_ignore_ascii_case("MINID") {
            i += 1;
            if i < rest.len() && (rest[i] == "~" || rest[i] == "=") {
                i += 1;
            }
            if i >= rest.len() {
                return resp::write_err(out, "syntax error");
            }
            i += 1;
        } else {
            break;
        }
    }

    if i >= rest.len() {
        return resp::write_wrong_args(out, "xadd");
    }
    let id_str = rest[i];
    i += 1;

    let field_parts = &rest[i..];
    if field_parts.is_empty() || field_parts.len() % 2 != 0 {
        return resp::write_wrong_args(out, "xadd");
    }

    let fields: Vec<(String, String)> = field_parts
        .chunks(2)
        .map(|c| (c[0].to_string(), c[1].to_string()))
        .collect();

    match store.xadd(key, id_str, fields, maxlen, nomkstream) {
        Ok(Some(id)) => resp::write_bulk(out, &id),
        Ok(None) => resp::write_nil(out),
        Err(e) => resp::write_store_err(out, e),
    }
}

pub fn xlen(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key] => resp::write_integer(out, wt!(out, store.xlen(key)) as i64),
        _ => resp::write_wrong_args(out, "xlen"),
    }
}

pub fn xrange(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let [_, key, start, end, rest @ ..] = parts else {
        return resp::write_wrong_args(out, "xrange");
    };
    let mut count = 0usize;
    let mut i = 0;
    while i < rest.len() {
        if rest[i].eq_ignore_ascii_case("COUNT") {
            i += 1;
            if i >= rest.len() {
                return resp::write_err(out, "syntax error");
            }
            count = parse_int!(out, rest[i], usize);
        }
        i += 1;
    }
    let items = wt!(out, store.xrange(key, start, end, count));
    write_stream_entries(out, &items);
}

pub fn xrevrange(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let [_, key, end, start, rest @ ..] = parts else {
        return resp::write_wrong_args(out, "xrevrange");
    };
    let mut count = 0usize;
    let mut i = 0;
    while i < rest.len() {
        if rest[i].eq_ignore_ascii_case("COUNT") {
            i += 1;
            if i >= rest.len() {
                return resp::write_err(out, "syntax error");
            }
            count = parse_int!(out, rest[i], usize);
        }
        i += 1;
    }
    let items = wt!(out, store.xrevrange(key, end, start, count));
    write_stream_entries(out, &items);
}

pub fn xtrim(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let [_, key, rest @ ..] = parts else {
        return resp::write_wrong_args(out, "xtrim");
    };
    let mut i = 0;
    let mut maxlen = 0usize;
    while i < rest.len() {
        if rest[i].eq_ignore_ascii_case("MAXLEN") {
            i += 1;
            if i < rest.len() && (rest[i] == "~" || rest[i] == "=") {
                i += 1;
            }
            if i >= rest.len() {
                return resp::write_err(out, "syntax error");
            }
            maxlen = parse_int!(out, rest[i], usize);
        }
        i += 1;
    }
    resp::write_integer(out, store_ok!(out, store.xtrim(key, maxlen)) as i64);
}

pub fn xdel(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key, ids @ ..] if !ids.is_empty() => {
            resp::write_integer(out, store_ok!(out, store.xdel(key, ids)) as i64);
        }
        _ => resp::write_wrong_args(out, "xdel"),
    }
}

pub fn xgroup(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let [_, sub, rest @ ..] = parts else {
        return resp::write_wrong_args(out, "xgroup");
    };

    if sub.eq_ignore_ascii_case("CREATE") {
        let [key, group, id, opts @ ..] = rest else {
            return resp::write_wrong_args(out, "xgroup create");
        };
        let mkstream = opts.iter().any(|o| o.eq_ignore_ascii_case("MKSTREAM"));
        match store.xgroup_create(key, group, id, mkstream) {
            Ok(_) => resp::write_ok(out),
            Err(e) => resp::write_store_err(out, e),
        }
    } else if sub.eq_ignore_ascii_case("DESTROY") {
        match rest {
            [key, group] => {
                resp::write_boolean(out, store_ok!(out, store.xgroup_destroy(key, group)));
            }
            _ => resp::write_wrong_args(out, "xgroup destroy"),
        }
    } else if sub.eq_ignore_ascii_case("DELCONSUMER") {
        resp::write_integer(out, 0);
    } else if sub.eq_ignore_ascii_case("SETID") {
        resp::write_ok(out);
    } else {
        resp::write_err(out, "unknown XGROUP subcommand");
    }
}

pub fn xack(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let [_, key, group, ids @ ..] = parts else {
        return resp::write_wrong_args(out, "xack");
    };
    if ids.is_empty() {
        return resp::write_wrong_args(out, "xack");
    }
    resp::write_integer(out, store_ok!(out, store.xack(key, group, ids)) as i64);
}

pub fn xinfo(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let [_, sub, rest @ ..] = parts else {
        return resp::write_wrong_args(out, "xinfo");
    };
    if sub.eq_ignore_ascii_case("STREAM") {
        let [key, ..] = rest else {
            return resp::write_wrong_args(out, "xinfo stream");
        };
        match store.xlen(key) {
            Ok(len) => {
                resp::write_array_header(out, 4);
                resp::write_bulk(out, "length");
                resp::write_integer(out, len as i64);
                resp::write_bulk(out, "groups");
                resp::write_integer(out, 0);
            }
            Err(e) => resp::write_store_err(out, e),
        }
    } else if sub.eq_ignore_ascii_case("GROUPS")
        || sub.eq_ignore_ascii_case("CONSUMERS")
    {
        resp::write_array_header(out, 0);
    } else {
        resp::write_err(out, "unknown XINFO subcommand");
    }
}

pub fn xread(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    let mut count = 0usize;
    let mut i = 1;
    let mut streams_idx = 0;

    while i < parts.len() {
        if parts[i].eq_ignore_ascii_case("COUNT") {
            i += 1;
            if i >= parts.len() {
                return resp::write_err(out, "syntax error");
            }
            count = parse_int!(out, parts[i], usize);
        } else if parts[i].eq_ignore_ascii_case("BLOCK") {
            i += 1;
        } else if parts[i].eq_ignore_ascii_case("STREAMS") {
            streams_idx = i + 1;
            break;
        }
        i += 1;
    }

    if streams_idx == 0 || streams_idx >= parts.len() {
        return resp::write_wrong_args(out, "xread");
    }

    let stream_args = &parts[streams_idx..];
    if !stream_args.len().is_multiple_of(2) {
        return resp::write_wrong_args(out, "xread");
    }
    let half = stream_args.len() / 2;
    let keys = &stream_args[..half];
    let ids = &stream_args[half..];

    let mut any_data = false;
    let mut results: Vec<(&str, Vec<crate::storage::stream::StreamEntry>)> = Vec::new();
    for (k_idx, &key) in keys.iter().enumerate() {
        let start = if ids[k_idx] == "$" { "+" } else { ids[k_idx] };
        let entries = store.xrange(key, start, "+", count).unwrap_or_default();
        if !entries.is_empty() {
            any_data = true;
        }
        results.push((key, entries));
    }

    if !any_data {
        resp::write_nil(out);
        return;
    }

    resp::write_array_header(out, results.len());
    for (key, entries) in &results {
        resp::write_array_header(out, 2);
        resp::write_bulk(out, key);
        write_stream_entries(out, entries);
    }
}

fn write_stream_entries(out: &mut Vec<u8>, items: &[(String, Vec<(String, String)>)]) {
    resp::write_array_header(out, items.len());
    for (id, fields) in items {
        resp::write_array_header(out, 2);
        resp::write_bulk(out, id);
        resp::write_array_header(out, fields.len() * 2);
        for (f, v) in fields {
            resp::write_bulk(out, f);
            resp::write_bulk(out, v);
        }
    }
}
