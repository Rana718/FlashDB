use std::sync::Arc;

use crate::commends;
use crate::pubsub::encode_sub_reply;
use crate::utils::resp;

use super::conn::{Conn, ConnMode};
use super::pubsub_cmds::pubsub_info;
use super::subscription::{
    handle_psubscribe, handle_punsubscribe, handle_subscribe, handle_unsubscribe,
};

pub fn dispatch(conn: &mut Conn, parts: &[&str]) {
    if parts.is_empty() {
        conn.parser
            .wbuf
            .extend_from_slice(b"-ERR empty command\r\n");
        return;
    }

    let cmd = parts[0].as_bytes();

    match &conn.mode {
        ConnMode::Normal => {
            match cmd.first().map(|b| b.to_ascii_uppercase()) {
                Some(b'S') if cmd_eq(cmd, b"SET") => {
                    return commends::string::set(parts, &conn.store, &mut conn.parser.wbuf);
                }
                Some(b'G') if cmd_eq(cmd, b"GET") => {
                    return commends::string::get(parts, &conn.store, &mut conn.parser.wbuf);
                }
                _ => {}
            }

            if cmd_eq(cmd, b"SUBSCRIBE") {
                handle_subscribe(conn, parts);
            } else if cmd_eq(cmd, b"PSUBSCRIBE") {
                handle_psubscribe(conn, parts);
            } else if cmd_eq(cmd, b"UNSUBSCRIBE") || cmd_eq(cmd, b"PUNSUBSCRIBE") {
                conn.parser
                    .wbuf
                    .extend_from_slice(&encode_sub_reply("unsubscribe", "", 0));
            } else if cmd_eq(cmd, b"PUBLISH") {
                match parts {
                    [_, channel, message] => {
                        let n = conn.pubsub.publish(channel, message);
                        resp::write_integer(&mut conn.parser.wbuf, n as i64);
                    }
                    _ => resp::write_wrong_args(&mut conn.parser.wbuf, "publish"),
                }
            } else if cmd_eq(cmd, b"PUBSUB") {
                let pubsub = Arc::clone(&conn.pubsub);
                pubsub_info(parts, &pubsub, &mut conn.parser.wbuf);
            } else {
                commends::execute(parts, &conn.store, &mut conn.parser.wbuf);
            }
        }

        ConnMode::Subscribed { .. } => {
            if cmd_eq(cmd, b"SUBSCRIBE") {
                handle_subscribe(conn, parts);
            } else if cmd_eq(cmd, b"UNSUBSCRIBE") {
                handle_unsubscribe(conn, parts);
            } else if cmd_eq(cmd, b"PSUBSCRIBE") {
                handle_psubscribe(conn, parts);
            } else if cmd_eq(cmd, b"PUNSUBSCRIBE") {
                handle_punsubscribe(conn, parts);
            } else if cmd_eq(cmd, b"PING") {
                let out = &mut conn.parser.wbuf;
                let msg = parts.get(1).copied().unwrap_or("");
                if msg.is_empty() {
                    out.extend_from_slice(b"*2\r\n$4\r\npong\r\n$0\r\n\r\n");
                } else {
                    out.extend_from_slice(b"*2\r\n$4\r\npong\r\n");
                    resp::write_bulk(out, msg);
                }
            } else if cmd_eq(cmd, b"RESET") || cmd_eq(cmd, b"QUIT") {
                super::subscription::do_full_unsubscribe(conn);
                resp::write_simple(&mut conn.parser.wbuf, "OK");
            } else {
                conn.parser
                    .wbuf
                    .extend_from_slice(b"-ERR Command not allowed in subscribed state\r\n");
            }
        }
    }
}

#[inline(always)]
pub fn cmd_eq(a: &[u8], upper: &[u8]) -> bool {
    a.len() == upper.len()
        && a.iter()
            .zip(upper.iter())
            .all(|(&ac, &uc)| ac.to_ascii_uppercase() == uc)
}
