use crate::storage::store::Store;
use crate::utils::resp;

pub fn ping(parts: &[String], out: &mut Vec<u8>) {
    match parts {
        [_] => out.extend_from_slice(resp::PONG),
        [_, msg] => resp::write_bulk(out, msg),
        _ => resp::write_wrong_args(out, "ping"),
    }
}

pub fn echo(parts: &[String], out: &mut Vec<u8>) {
    match parts {
        [_, msg] => resp::write_bulk(out, msg),
        _ => resp::write_wrong_args(out, "echo"),
    }
}

pub fn info(store: &Store, out: &mut Vec<u8>) {
    resp::write_bulk(out, &store.info());
}

pub fn flush(store: &Store, out: &mut Vec<u8>) {
    store.flush();
    resp::write_ok(out);
}

pub fn dbsize(store: &Store, out: &mut Vec<u8>) {
    resp::write_integer(out, store.dbsize() as i64);
}

pub fn type_of(parts: &[String], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key] => resp::write_simple(out, store.type_of(key)),
        _ => resp::write_wrong_args(out, "type"),
    }
}
