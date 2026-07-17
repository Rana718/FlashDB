pub const OK: &str = "+OK\r\n";
pub const NIL: &str = "$-1\r\n";
pub const ZERO: &str = ":0\r\n";
pub const ONE: &str = ":1\r\n";
pub const PONG: &str = "+PONG\r\n";

#[inline]
pub fn bulk(s: &str) -> String {
    format!("${}\r\n{}\r\n", s.len(), s)
}

#[inline]
pub fn integer(n: i64) -> String {
    format!(":{}\r\n", n)
}

#[inline]
pub fn boolean(b: bool) -> String {
    if b { ONE.into() } else { ZERO.into() }
}

#[inline]
pub fn opt_bulk(v: Option<String>) -> String {
    match v {
        Some(s) => bulk(&s),
        None => NIL.into(),
    }
}

#[inline]
pub fn array(items: &[String]) -> String {
    let mut out = format!("*{}\r\n", items.len());
    for item in items {
        out.push_str(&bulk(item));
    }
    out
}

#[inline]
pub fn opt_array(items: &[Option<String>]) -> String {
    let mut out = format!("*{}\r\n", items.len());
    for item in items {
        match item {
            Some(s) => out.push_str(&bulk(s)),
            None => out.push_str(NIL),
        }
    }
    out
}

#[inline]
pub fn err(msg: &str) -> String {
    format!("-ERR {}\r\n", msg)
}

#[inline]
pub fn wrong_type() -> String {
    "-WRONGTYPE Operation against a key holding the wrong kind of value\r\n".into()
}

#[inline]
pub fn wrong_args(cmd: &str) -> String {
    format!("-ERR wrong number of arguments for '{}' command\r\n", cmd)
}
