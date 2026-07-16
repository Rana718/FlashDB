
use crate::storage::store::Store;

pub async fn ping(parts: Vec<String>) -> String {
    match parts.as_slice(){
       [_] => "+PONG\r\n".to_string(),
       _ => "ERR wrong number of arguments for 'ping' command\r\n".to_string(),
    }
}

pub async fn echo(parts: Vec<String>) -> String {
    match parts.as_slice(){
        [_] => parts[0].clone(),
        _ => "ERR wrong number of arguments for 'echo' command\r\n".to_string(),
    }
}

pub async fn info(store: &Store) -> String {
    let info = store.info();

    format!(
        "${}\r\n{}\r\n",
        info.len(),
        info
    )
}

pub async fn flush(store: &Store) -> String {
    store.flush();
    "+OK\r\n".to_string()
}