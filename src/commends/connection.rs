use crate::utils::resp::{self, OK, PONG};
use crate::storage::store::Store;

pub async fn ping(parts: Vec<String>) -> String {
    match parts.as_slice() {
        [_] => PONG.into(),
        [_, msg] => resp::bulk(msg),
        _ => resp::wrong_args("ping"),
    }
}

pub async fn echo(parts: Vec<String>) -> String {
    match parts.as_slice() {
        [_, msg] => resp::bulk(msg),
        _ => resp::wrong_args("echo"),
    }
}

pub async fn info(store: &Store) -> String {
    resp::bulk(&store.info())
}

pub async fn flush(store: &Store) -> String {
    store.flush();
    OK.into()
}

pub async fn dbsize(store: &Store) -> String {
    resp::integer(store.dbsize() as i64)
}

pub async fn type_of(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key] => format!("+{}\r\n", store.type_of(key)),
        _ => resp::wrong_args("type"),
    }
}
