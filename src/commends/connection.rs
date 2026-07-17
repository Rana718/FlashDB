
use crate::storage::store::Store;

pub async fn ping(parts: Vec<String>) -> String {
    match parts.as_slice() {
        [_] => "+PONG\r\n".to_string(),
        [_, msg] => format!("${}\r\n{}\r\n", msg.len(), msg),
        _ => "-ERR wrong number of arguments for 'ping' command\r\n".to_string(),
    }
}

pub async fn echo(parts: Vec<String>) -> String {
    match parts.as_slice() {
        [_, msg] => format!("${}\r\n{}\r\n", msg.len(), msg),
        _ => "-ERR wrong number of arguments for 'echo' command\r\n".to_string(),
    }
}

pub async fn info(store: &Store) -> String {
    let info = store.info();
    format!("${}\r\n{}\r\n", info.len(), info)
}

pub async fn flush(store: &Store) -> String {
    store.flush();
    "+OK\r\n".to_string()
}

pub async fn dbsize(store: &Store) -> String {
    format!(":{}\r\n", store.dbsize())
}

pub async fn type_of(parts: Vec<String>, store: &Store) -> String {
    match parts.as_slice() {
        [_, key] => format!("+{}\r\n", store.type_of(key)),
        _ => "-ERR wrong number of arguments for 'type' command\r\n".to_string(),
    }
}