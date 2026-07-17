use crate::string_enum;

use super::storage::store::Store;

string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ComdType {
        default = Exec;
        Exec => "EXEC",
        PING => "PING",
        ECHO => "ECHO",
        INFO => "INFO",
        FLUSH => "FLUSH",
        DBSIZE => "DBSIZE",
        TYPE => "TYPE",
        
        GET => "GET",
        SET => "SET",
        INCR => "INCR",
        DECR => "DECR",
        MSET => "MSET",
        MGET => "MGET",
        
        DEL => "DEL",
        EXISTS => "EXISTS",
        TTL => "TTL",
        EXPIRE => "EXPIRE",
        SCAN => "SCAN",
        
    }
}

pub async fn execute(parts: Vec<String>, store: &Store) -> String {
    if parts.is_empty() {
        return "-ERR invalid command\r\n".into();
    }

    let cmd = ComdType::from(parts[0].as_str());
    match cmd {
        // Connection commands
        ComdType::ECHO => connection::echo(parts).await,
        ComdType::PING => connection::ping(parts).await,
        ComdType::INFO => connection::info(store).await,
        ComdType::FLUSH => connection::flush(store).await,
        ComdType::DBSIZE => connection::dbsize(store).await,
        ComdType::TYPE => connection::type_of(parts, store).await,

        // String commands
        ComdType::GET => string::get(parts, store).await,
        ComdType::SET => string::set(parts, store).await,
        ComdType::INCR => string::incr(parts, store).await,
        ComdType::MSET => string::mset(parts, store).await,
        ComdType::MGET => string::mget(parts, store).await,
        ComdType::DECR => string::decr(parts, store).await,

        // Key commands
        ComdType::DEL => keys::del(parts, store).await,
        ComdType::EXISTS => keys::exists_check(parts, store).await,
        ComdType::TTL => keys::ttl_check(parts, store).await,
        ComdType::EXPIRE => keys::expire(parts, store).await,
        ComdType::SCAN => scan::scan(parts, store).await,
        _ => "-ERR unknown command\r\n".into(),
    }
}

pub mod connection;
pub mod keys;
pub mod macros;
pub mod scan;
pub mod string;
