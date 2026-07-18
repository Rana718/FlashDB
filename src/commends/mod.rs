use super::storage::store::Store;
use crate::string_enum;

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

pub fn execute(parts: &[String], store: &Store) -> String {
    if parts.is_empty() {
        return "-ERR invalid command\r\n".into();
    }

    let cmd = ComdType::from(parts[0].as_str());
    match cmd {
        // Connection
        ComdType::PING => connection::ping(parts),
        ComdType::ECHO => connection::echo(parts),
        ComdType::INFO => connection::info(store),
        ComdType::FLUSH => connection::flush(store),
        ComdType::DBSIZE => connection::dbsize(store),
        ComdType::TYPE => connection::type_of(parts, store),

        // String
        ComdType::GET => string::get(parts, store),
        ComdType::SET => string::set(parts, store),
        ComdType::SETNX => string::setnx(parts, store),
        ComdType::SETEX => string::setex(parts, store),
        ComdType::PSETEX => string::psetex(parts, store),
        ComdType::GETDEL => string::getdel(parts, store),
        ComdType::GETSET => string::getset(parts, store),
        ComdType::GETEX => string::getex(parts, store),
        ComdType::MSET => string::mset(parts, store),
        ComdType::MSETNX => string::msetnx(parts, store),
        ComdType::MGET => string::mget(parts, store),
        ComdType::INCR => string::incr(parts, store),
        ComdType::DECR => string::decr(parts, store),
        ComdType::INCRBY => string::incrby(parts, store),
        ComdType::DECRBY => string::decrby(parts, store),
        ComdType::INCRBYFLOAT => string::incrbyfloat(parts, store),
        ComdType::APPEND => string::append(parts, store),
        ComdType::STRLEN => string::strlen(parts, store),
        ComdType::GETRANGE => string::getrange(parts, store),
        ComdType::SETRANGE => string::setrange(parts, store),

        // Keys
        ComdType::DEL => keys::del(parts, store),
        ComdType::UNLINK => keys::unlink(parts, store),
        ComdType::EXISTS => keys::exists_check(parts, store),
        ComdType::TTL => keys::ttl_check(parts, store),
        ComdType::PTTL => keys::pttl_check(parts, store),
        ComdType::EXPIRE => keys::expire(parts, store),
        ComdType::PEXPIRE => keys::pexpire(parts, store),
        ComdType::EXPIREAT => keys::expireat(parts, store),
        ComdType::PERSIST => keys::persist(parts, store),
        ComdType::RENAME => keys::rename(parts, store),
        ComdType::RENAMENX => keys::renamenx(parts, store),
        ComdType::COPY => keys::copy(parts, store),
        ComdType::RANDOMKEY => keys::randomkey(parts, store),
        ComdType::KEYS => keys::keys(parts, store),
        ComdType::SCAN => scan::scan(parts, store),

        // Hash
        ComdType::HSET => hash::hset(parts, store),
        ComdType::HSETNX => hash::hsetnx(parts, store),
        ComdType::HGET => hash::hget(parts, store),
        ComdType::HMGET => hash::hmget(parts, store),
        ComdType::HMSET => hash::hmset(parts, store),
        ComdType::HGETALL => hash::hgetall(parts, store),
        ComdType::HDEL => hash::hdel(parts, store),
        ComdType::HEXISTS => hash::hexists(parts, store),
        ComdType::HLEN => hash::hlen(parts, store),
        ComdType::HKEYS => hash::hkeys(parts, store),
        ComdType::HVALS => hash::hvals(parts, store),
        ComdType::HINCRBY => hash::hincrby(parts, store),
        ComdType::HINCRBYFLOAT => hash::hincrbyfloat(parts, store),

        _ => "-ERR unknown command\r\n".into(),
    }
}

pub mod connection;
pub mod hash;
pub mod keys;
pub mod scan;
pub mod string;
