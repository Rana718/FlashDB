use crate::commends;
use crate::storage::store::Store;
use crate::utils::parser::{ParseResult, RespParser};
use mio::net::TcpStream;
use std::io::{self, Read, Write};
use std::sync::Arc;

pub struct Conn {
    pub stream: TcpStream,
    pub parser: RespParser,
    pub store: Arc<Store>,
    pub write_offset: usize,
}

impl Conn {
    pub fn new(stream: TcpStream, store: Arc<Store>) -> Self {
        store.client_connected();
        Self {
            stream,
            parser: RespParser::new(),
            store,
            write_offset: 0,
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
                    let parts_raw = self.parser.parts_raw.as_slice();
                    let wbuf = &mut self.parser.wbuf;
                    commends::execute_raw(parts_raw, &self.store, wbuf);
                }
                ParseResult::Incomplete => break,
                ParseResult::Error => return false,
            }
        }

        true
    }

    pub fn do_write(&mut self) -> bool {
        let wbuf = &self.parser.wbuf;
        if wbuf.is_empty() {
            return true;
        }
        loop {
            match self.stream.write(&wbuf[self.write_offset..]) {
                Ok(n) => {
                    self.write_offset += n;
                    if self.write_offset >= wbuf.len() {
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
}

impl Drop for Conn {
    fn drop(&mut self) {
        self.store.client_disconnected();
    }
}
