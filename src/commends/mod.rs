use crate::string_enum;
use super::storage::store::Store;

string_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ComdType {
        default = Exec;
        Exec => "EXEC",

        // Connection
        PING => "PING",
        ECHO => "ECHO",
        INFO => "INFO",
        FLUSH => "FLUSH",
        DBSIZE => "DBSIZE",
        TYPE => "TYPE",

        // String
        GET => "GET",
        SET => "SET",
        SETNX => "SETNX",
        SETEX => "SETEX",
        PSETEX => "PSETEX",
        GETDEL => "GETDEL",
        GETSET => "GETSET",
        GETEX => "GETEX",
        MSET => "MSET",
        MSETNX => "MSETNX",
        MGET => "MGET",
        INCR => "INCR",
        DECR => "DECR",
        INCRBY => "INCRBY",
        DECRBY => "DECRBY",
        INCRBYFLOAT => "INCRBYFLOAT",
        APPEND => "APPEND",
        STRLEN => "STRLEN",
        GETRANGE => "GETRANGE",
        SETRANGE => "SETRANGE",

        // Keys
        DEL => "DEL",
        UNLINK => "UNLINK",
        EXISTS => "EXISTS",
        TTL => "TTL",
        PTTL => "PTTL",
        EXPIRE => "EXPIRE",
        PEXPIRE => "PEXPIRE",
        EXPIREAT => "EXPIREAT",
        PERSIST => "PERSIST",
        RENAME => "RENAME",
        RENAMENX => "RENAMENX",
        COPY => "COPY",
        RANDOMKEY => "RANDOMKEY",
        KEYS => "KEYS",
        SCAN => "SCAN",

        // Hash
        HSET => "HSET",
        HSETNX => "HSETNX",
        HGET => "HGET",
        HMGET => "HMGET",
        HMSET => "HMSET",
        HGETALL => "HGETALL",
        HDEL => "HDEL",
        HEXISTS => "HEXISTS",
        HLEN => "HLEN",
        HKEYS => "HKEYS",
        HVALS => "HVALS",
        HINCRBY => "HINCRBY",
        HINCRBYFLOAT => "HINCRBYFLOAT",
    }
}

pub async fn execute(parts: Vec<String>, store: &Store) -> String {
    if parts.is_empty() {
        return "-ERR invalid command\r\n".into();
    }

    let cmd = ComdType::from(parts[0].as_str());
    match cmd {
        // Connection
        ComdType::PING => connection::ping(parts).await,
        ComdType::ECHO => connection::echo(parts).await,
        ComdType::INFO => connection::info(store).await,
        ComdType::FLUSH => connection::flush(store).await,
        ComdType::DBSIZE => connection::dbsize(store).await,
        ComdType::TYPE => connection::type_of(parts, store).await,

        // String
        ComdType::GET => string::get(parts, store).await,
        ComdType::SET => string::set(parts, store).await,
        ComdType::SETNX => string::setnx(parts, store).await,
        ComdType::SETEX => string::setex(parts, store).await,
        ComdType::PSETEX => string::psetex(parts, store).await,
        ComdType::GETDEL => string::getdel(parts, store).await,
        ComdType::GETSET => string::getset(parts, store).await,
        ComdType::GETEX => string::getex(parts, store).await,
        ComdType::MSET => string::mset(parts, store).await,
        ComdType::MSETNX => string::msetnx(parts, store).await,
        ComdType::MGET => string::mget(parts, store).await,
        ComdType::INCR => string::incr(parts, store).await,
        ComdType::DECR => string::decr(parts, store).await,
        ComdType::INCRBY => string::incrby(parts, store).await,
        ComdType::DECRBY => string::decrby(parts, store).await,
        ComdType::INCRBYFLOAT => string::incrbyfloat(parts, store).await,
        ComdType::APPEND => string::append(parts, store).await,
        ComdType::STRLEN => string::strlen(parts, store).await,
        ComdType::GETRANGE => string::getrange(parts, store).await,
        ComdType::SETRANGE => string::setrange(parts, store).await,

        // Keys
        ComdType::DEL => keys::del(parts, store).await,
        ComdType::UNLINK => keys::unlink(parts, store).await,
        ComdType::EXISTS => keys::exists_check(parts, store).await,
        ComdType::TTL => keys::ttl_check(parts, store).await,
        ComdType::PTTL => keys::pttl_check(parts, store).await,
        ComdType::EXPIRE => keys::expire(parts, store).await,
        ComdType::PEXPIRE => keys::pexpire(parts, store).await,
        ComdType::EXPIREAT => keys::expireat(parts, store).await,
        ComdType::PERSIST => keys::persist(parts, store).await,
        ComdType::RENAME => keys::rename(parts, store).await,
        ComdType::RENAMENX => keys::renamenx(parts, store).await,
        ComdType::COPY => keys::copy(parts, store).await,
        ComdType::RANDOMKEY => keys::randomkey(parts, store).await,
        ComdType::KEYS => keys::keys(parts, store).await,
        ComdType::SCAN => scan::scan(parts, store).await,

        // Hash
        ComdType::HSET => hash::hset(parts, store).await,
        ComdType::HSETNX => hash::hsetnx(parts, store).await,
        ComdType::HGET => hash::hget(parts, store).await,
        ComdType::HMGET => hash::hmget(parts, store).await,
        ComdType::HMSET => hash::hmset(parts, store).await,
        ComdType::HGETALL => hash::hgetall(parts, store).await,
        ComdType::HDEL => hash::hdel(parts, store).await,
        ComdType::HEXISTS => hash::hexists(parts, store).await,
        ComdType::HLEN => hash::hlen(parts, store).await,
        ComdType::HKEYS => hash::hkeys(parts, store).await,
        ComdType::HVALS => hash::hvals(parts, store).await,
        ComdType::HINCRBY => hash::hincrby(parts, store).await,
        ComdType::HINCRBYFLOAT => hash::hincrbyfloat(parts, store).await,

        _ => "-ERR unknown command\r\n".into(),
    }
}

pub mod connection;
pub mod hash;
pub mod keys;
pub mod scan;
pub mod string;
