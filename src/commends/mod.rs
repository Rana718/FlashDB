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
        BGSAVE => "BGSAVE",

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

pub fn execute_raw(parts_raw: &[(*const u8, usize)], store: &Store, out: &mut Vec<u8>) {
    const STACK_CAP: usize = 32;
    if parts_raw.len() <= STACK_CAP {
        let mut arr = [""; STACK_CAP];
        for (i, &(ptr, len)) in parts_raw.iter().enumerate() {
            arr[i] = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len)) };
        }
        execute(&arr[..parts_raw.len()], store, out);
    } else {
        let parts: Vec<&str> = parts_raw
            .iter()
            .map(|&(ptr, len)| unsafe {
                std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len))
            })
            .collect();
        execute(&parts, store, out);
    }
}

pub fn execute(parts: &[&str], store: &Store, out: &mut Vec<u8>) {
    if parts.is_empty() {
        out.extend_from_slice(b"-ERR invalid command\r\n");
        return;
    }

    let cmd = ComdType::from(parts[0]);
    match cmd {
        // Connection
        ComdType::PING => connection::ping(parts, out),
        ComdType::ECHO => connection::echo(parts, out),
        ComdType::INFO => connection::info(store, out),
        ComdType::FLUSH => connection::flush(store, out),
        ComdType::DBSIZE => connection::dbsize(store, out),
        ComdType::TYPE => connection::type_of(parts, store, out),
        ComdType::BGSAVE => connection::bgsave(store, out),

        // String
        ComdType::GET => string::get(parts, store, out),
        ComdType::SET => string::set(parts, store, out),
        ComdType::SETNX => string::setnx(parts, store, out),
        ComdType::SETEX => string::setex(parts, store, out),
        ComdType::PSETEX => string::psetex(parts, store, out),
        ComdType::GETDEL => string::getdel(parts, store, out),
        ComdType::GETSET => string::getset(parts, store, out),
        ComdType::GETEX => string::getex(parts, store, out),
        ComdType::MSET => string::mset(parts, store, out),
        ComdType::MSETNX => string::msetnx(parts, store, out),
        ComdType::MGET => string::mget(parts, store, out),
        ComdType::INCR => string::incr(parts, store, out),
        ComdType::DECR => string::decr(parts, store, out),
        ComdType::INCRBY => string::incrby(parts, store, out),
        ComdType::DECRBY => string::decrby(parts, store, out),
        ComdType::INCRBYFLOAT => string::incrbyfloat(parts, store, out),
        ComdType::APPEND => string::append(parts, store, out),
        ComdType::STRLEN => string::strlen(parts, store, out),
        ComdType::GETRANGE => string::getrange(parts, store, out),
        ComdType::SETRANGE => string::setrange(parts, store, out),

        // Keys
        ComdType::DEL => keys::del(parts, store, out),
        ComdType::UNLINK => keys::unlink(parts, store, out),
        ComdType::EXISTS => keys::exists_check(parts, store, out),
        ComdType::TTL => keys::ttl_check(parts, store, out),
        ComdType::PTTL => keys::pttl_check(parts, store, out),
        ComdType::EXPIRE => keys::expire(parts, store, out),
        ComdType::PEXPIRE => keys::pexpire(parts, store, out),
        ComdType::EXPIREAT => keys::expireat(parts, store, out),
        ComdType::PERSIST => keys::persist(parts, store, out),
        ComdType::RENAME => keys::rename(parts, store, out),
        ComdType::RENAMENX => keys::renamenx(parts, store, out),
        ComdType::COPY => keys::copy(parts, store, out),
        ComdType::RANDOMKEY => keys::randomkey(parts, store, out),
        ComdType::KEYS => keys::keys(parts, store, out),
        ComdType::SCAN => scan::scan(parts, store, out),

        // Hash
        ComdType::HSET => hash::hset(parts, store, out),
        ComdType::HSETNX => hash::hsetnx(parts, store, out),
        ComdType::HGET => hash::hget(parts, store, out),
        ComdType::HMGET => hash::hmget(parts, store, out),
        ComdType::HMSET => hash::hmset(parts, store, out),
        ComdType::HGETALL => hash::hgetall(parts, store, out),
        ComdType::HDEL => hash::hdel(parts, store, out),
        ComdType::HEXISTS => hash::hexists(parts, store, out),
        ComdType::HLEN => hash::hlen(parts, store, out),
        ComdType::HKEYS => hash::hkeys(parts, store, out),
        ComdType::HVALS => hash::hvals(parts, store, out),
        ComdType::HINCRBY => hash::hincrby(parts, store, out),
        ComdType::HINCRBYFLOAT => hash::hincrbyfloat(parts, store, out),

        _ => out.extend_from_slice(b"-ERR unknown command\r\n"),
    }
}

pub mod connection;
pub mod hash;
pub mod keys;
pub mod scan;
pub mod string;
