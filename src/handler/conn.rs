use mio::net::TcpStream;
use std::collections::HashSet;
use std::io::{self, Read, Write};
use std::sync::Arc;

use crate::pubsub::{PubSub, SubSlot, WorkerNotifier};
use crate::storage::store::Store;
use crate::utils::parser::{ParseResult, RespParser};

use super::dispatch::dispatch;
use super::subscription::do_full_unsubscribe;

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

        // Process all parsed commands in a tight batch
        loop {
            match self.parser.parse_one() {
                ParseResult::Complete => {
                    let raw = self.parser.parts_as_raw();
                    dispatch_raw(self, &raw);
                }
                ParseResult::Incomplete => break,
                ParseResult::Error => return false,
            }
        }

        true
    }

    pub fn do_write(&mut self) -> bool {
        if let ConnMode::Subscribed { ref slot, .. } = self.mode {
            slot.drain_into(&mut self.parser.wbuf);
        }

        if self.parser.wbuf.is_empty() {
            return true;
        }

        loop {
            match self.stream.write(&self.parser.wbuf[self.write_offset..]) {
                Ok(n) => {
                    self.write_offset += n;
                    if self.write_offset >= self.parser.wbuf.len() {
                        self.write_offset = 0;
                        self.parser.wbuf.clear();
                        return true;
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return true,
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
fn dispatch_raw(conn: &mut Conn, raw: &[(*const u8, usize)]) {
    const STACK_CAP: usize = 32;
    if raw.len() <= STACK_CAP {
        let mut arr = [""; STACK_CAP];
        for (i, &(ptr, len)) in raw.iter().enumerate() {
            arr[i] = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len)) };
        }
        dispatch(conn, &arr[..raw.len()]);
    } else {
        let parts: Vec<&str> = raw
            .iter()
            .map(|&(ptr, len)| unsafe {
                std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len))
            })
            .collect();
        dispatch(conn, &parts);
    }
}
