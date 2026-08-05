use crate::storage::rdb;
use crate::storage::store::Store;
use crate::utils::resp;

pub fn ping(parts: &[&str], out: &mut Vec<u8>) {
    match parts {
        [_] => out.extend_from_slice(resp::PONG),
        [_, msg] => resp::write_bulk(out, msg),
        _ => resp::write_wrong_args(out, "ping"),
    }
}

pub fn echo(parts: &[&str], out: &mut Vec<u8>) {
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

pub fn type_of(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    match parts {
        [_, key] => resp::write_simple(out, store.type_of(key)),
        _ => resp::write_wrong_args(out, "type"),
    }
}

pub fn bgsave(store: &Store, out: &mut Vec<u8>) {
    let store = store.clone();
    std::thread::spawn(move || match rdb::save(&store, "flashdb.rdb") {
        Ok(()) => eprintln!("[rdb] BGSAVE complete"),
        Err(e) => eprintln!("[rdb] BGSAVE error: {e}"),
    });
    resp::write_simple(out, "Background saving started");
}
