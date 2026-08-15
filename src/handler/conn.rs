use mio::net::TcpStream;
use std::collections::HashSet;
use std::io::{self, Read, Write};
use std::sync::Arc;

use crate::pubsub::{PubSub, SubSlot, WorkerNotifier};
use crate::storage::store::Store;
use crate::utils::parser::{ParseResult, RespParser};

use super::dispatch::dispatch;
use super::subscription::do_full_unsubscribe;

const SUB_WRITE_BATCH_BYTES: usize = 256 * 1024;
const RETAINED_WRITE_BUFFER: usize = 1024 * 1024;

pub enum ConnMode {
    Normal,
    Subscribed {
        slot: Arc<SubSlot>,
        channels: HashSet<String>,
        patterns: HashSet<String>,
    },
}

pub struct Conn {
    pub stream: TcpStream,
    pub parser: RespParser,
    pub store: Arc<Store>,
    pub pubsub: Arc<PubSub>,
    pub write_offset: usize,
    pub mode: ConnMode,
    pub token: usize,
    pub notifier: Arc<WorkerNotifier>,
}

impl Conn {
    pub fn new(
        stream: TcpStream,
        store: Arc<Store>,
        pubsub: Arc<PubSub>,
        token: usize,
        notifier: Arc<WorkerNotifier>,
    ) -> Self {
        store.client_connected();
        Self {
            stream,
            parser: RespParser::new(),
            store,
            pubsub,
            write_offset: 0,
            mode: ConnMode::Normal,
            token,
            notifier,
        }
    }

    pub fn do_read(&mut self) -> bool {
        loop {
            let buf = self.parser.read_buf();
            match self.stream.read(buf) {
                Ok(0) => return false,
                Ok(n) => self.parser.did_fill(n),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => return false,
            }
        }

        loop {
            match self.parser.parse_one() {
                ParseResult::Complete => {
                    let raw_ptr = self.parser.parts_raw.as_ptr();
                    let raw_len = self.parser.parts_raw.len();
                    let raw = unsafe { std::slice::from_raw_parts(raw_ptr, raw_len) };
                    dispatch_raw(self, raw);
                }
                ParseResult::Incomplete => break,
                ParseResult::Error => return false,
            }
        }

        true
    }

    pub fn do_write(&mut self) -> bool {
        if let ConnMode::Subscribed { ref slot, .. } = self.mode
            && self.parser.wbuf.len() < SUB_WRITE_BATCH_BYTES
        {
            slot.drain_into_limit(&mut self.parser.wbuf, SUB_WRITE_BATCH_BYTES);
        }

        if self.parser.wbuf.is_empty() {
            return true;
        }

        loop {
            match self.stream.write(&self.parser.wbuf[self.write_offset..]) {
                Ok(0) => return false,
                Ok(n) => {
                    self.write_offset += n;
                    if self.write_offset >= self.parser.wbuf.len() {
                        self.parser.wbuf.clear();
                        self.write_offset = 0;
                        if self.parser.wbuf.capacity() > RETAINED_WRITE_BUFFER {
                            self.parser.wbuf.shrink_to(RETAINED_WRITE_BUFFER);
                        }
                        return true;
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if self.write_offset > 0 {
                        self.parser.wbuf.drain(..self.write_offset);
                        self.write_offset = 0;
                    }
                    return true;
                }
                Err(_) => return false,
            }
        }
    }

    pub fn has_pending_write(&self) -> bool {
        !self.parser.wbuf.is_empty()
            || matches!(&self.mode, ConnMode::Subscribed { slot, .. } if slot.has_pending())
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        do_full_unsubscribe(self);
        self.store.client_disconnected();
    }
}

#[inline(always)]
unsafe fn part_bytes<'a>(part: (*const u8, usize)) -> &'a [u8] {
    unsafe { std::slice::from_raw_parts(part.0, part.1) }
}

#[inline(always)]
fn part_str<'a>(out: &mut Vec<u8>, part: (*const u8, usize)) -> Option<&'a str> {
    let bytes: &'a [u8] = unsafe { part_bytes(part) };
    match std::str::from_utf8(bytes) {
        Ok(s) => Some(s),
        Err(_) => {
            crate::utils::resp::write_err(out, "invalid UTF-8 in request");
            None
        }
    }
}

#[inline(always)]
fn dispatch_raw(conn: &mut Conn, raw: &[(*const u8, usize)]) {
    if raw.is_empty() {
        return;
    }

    let cmd_len = raw[0].1;
    let cmd: &[u8] = unsafe { part_bytes(raw[0]) };

    if cmd_len == 3 {
        if cmd.eq_ignore_ascii_case(b"SET") && raw.len() >= 3 {
            let out = &mut conn.parser.wbuf;
            let Some(key) = part_str(out, raw[1]) else {
                return;
            };
            let Some(value) = part_str(out, raw[2]) else {
                return;
            };
            if raw.len() == 3 {
                conn.store.set_string(key, value, 0);
                conn.parser.wbuf.extend_from_slice(b"+OK\r\n");
                return;
            }
        } else if cmd.eq_ignore_ascii_case(b"GET") && raw.len() == 2 {
            let out = &mut conn.parser.wbuf;
            let Some(key) = part_str(out, raw[1]) else {
                return;
            };
            if !conn.store.get_to_buf(key, &mut conn.parser.wbuf) {
                conn.parser.wbuf.extend_from_slice(b"$-1\r\n");
            }
            return;
        }
    }

    const STACK_CAP: usize = 32;
    if raw.len() <= STACK_CAP {
        let mut arr = [""; STACK_CAP];
        for (i, &part) in raw.iter().enumerate() {
            let Some(s) = part_str(&mut conn.parser.wbuf, part) else {
                return;
            };
            arr[i] = s;
        }
        dispatch(conn, &arr[..raw.len()]);
    } else {
        let mut parts: Vec<&str> = Vec::with_capacity(raw.len());
        for &part in raw.iter() {
            let Some(s) = part_str(&mut conn.parser.wbuf, part) else {
                return;
            };
            parts.push(s);
        }
        dispatch(conn, &parts);
    }
}
