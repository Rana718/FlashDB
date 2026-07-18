use crate::storage::store::Store;
use crate::utils::resp::{self, OK, PONG};

pub fn ping(parts: &[String]) -> String {
    match parts {
        [_] => PONG.into(),
        [_, msg] => resp::bulk(msg),
        _ => resp::wrong_args("ping"),
    }
}

pub fn echo(parts: &[String]) -> String {
    match parts {
        [_, msg] => resp::bulk(msg),
        _ => resp::wrong_args("echo"),
    }
}

pub fn info(store: &Store) -> String {
    resp::bulk(&store.info())
}

pub fn flush(store: &Store) -> String {
    store.flush();
    OK.into()
}

pub fn dbsize(store: &Store) -> String {
    resp::integer(store.dbsize() as i64)
}

pub fn type_of(parts: &[String], store: &Store) -> String {
    match parts {
        [_, key] => format!("+{}\r\n", store.type_of(key)),
        _ => resp::wrong_args("type"),
    }
}
