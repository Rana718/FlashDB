use crate::storage::rdb;
use crate::storage::store::Store;
use crate::utils::resp;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

static BGSAVE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

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

pub fn bgsave(store: &Arc<Store>, out: &mut Vec<u8>) {
    if BGSAVE_IN_PROGRESS.swap(true, Ordering::AcqRel) {
        return resp::write_err(out, "Background save already in progress");
    }
    let store = Arc::clone(store);
    let path = std::env::var("FLASHDB_RDB_PATH").unwrap_or_else(|_| "flashdb.rdb".to_string());
    std::thread::Builder::new()
        .name("flashdb-bgsave".into())
        .spawn(move || {
            match rdb::save(&store, &path) {
                Ok(()) => eprintln!("[rdb] BGSAVE complete"),
                Err(e) => eprintln!("[rdb] BGSAVE error: {e}"),
            }
            BGSAVE_IN_PROGRESS.store(false, Ordering::Release);
        })
        .ok();
    resp::write_simple(out, "Background saving started");
}
