use super::storage::store::Store;

pub async fn execute(parts: Vec<String>, store: &Store) -> String {
    if parts.is_empty() {
        return "-ERR invalid command\r\n".into();
    }

    match parts[0].to_ascii_uppercase().as_str() {
        // Connection commands
        "PING" => connection::ping(parts).await,
        "ECHO" => connection::echo(parts).await,

        // String commands
        "GET" => string::get(parts, store).await,
        "SET" => string::set(parts, store).await,

        // Key commands
        "DEL" => keys::del(parts, store).await,
        "EXISTS" => keys::exists_check(parts, store).await,
        "TTL" => keys::ttl_check(parts, store).await,

        _ => "-ERR unknown command\r\n".into(),
    }
}

pub mod connection;
pub mod keys;
pub mod string;
