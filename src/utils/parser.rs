pub struct RespParser {
    rbuf: Vec<u8>,
    filled: usize,
    pos: usize,
    pub wbuf: Vec<u8>,
    pub parts_buf: Vec<String>,
}

pub enum ParseResult<'a> {
    Complete(&'a [String]),
    Incomplete,
    Error,
}

impl RespParser {
    pub fn new() -> Self {
        Self {
            rbuf: vec![0u8; 65536],
            filled: 0,
            pos: 0,
            wbuf: Vec::with_capacity(65536),
            parts_buf: Vec::with_capacity(8),
        }
    }

    pub fn read_buf(&mut self) -> &mut [u8] {
        let remaining = self.rbuf.len() - self.filled;
        if remaining < 4096 {
            if self.pos > 0 {
                self.rbuf.copy_within(self.pos..self.filled, 0);
                self.filled -= self.pos;
                self.pos = 0;
            }
            if self.filled == self.rbuf.len() {
                self.rbuf.resize(self.rbuf.len() * 2, 0);
            }
        }
        &mut self.rbuf[self.filled..]
    }

    #[inline]
    pub fn did_fill(&mut self, n: usize) {
        self.filled += n;
    }

    pub fn parse_one(&mut self) -> ParseResult<'_> {
        self.parts_buf.clear();

        let start_pos = self.pos;

        let (s, e) = match self.scan_line() {
            Some(r) => r,
            None => { self.pos = start_pos; return ParseResult::Incomplete; }
        };

        if self.rbuf.get(s) != Some(&b'*') {
            return ParseResult::Error;
        }
        let count = match parse_usize(&self.rbuf[s + 1..e]) {
            Some(c) => c,
            None => return ParseResult::Error,
        };

        for _ in 0..count {
            let (bs, be) = match self.scan_line() {
                Some(r) => r,
                None => { self.pos = start_pos; return ParseResult::Incomplete; }
            };
            if self.rbuf.get(bs) != Some(&b'$') {
                return ParseResult::Error;
            }
            let len = match parse_usize(&self.rbuf[bs + 1..be]) {
                Some(l) => l,
                None => return ParseResult::Error,
            };
            if self.filled - self.pos < len + 2 {
                self.pos = start_pos;
                return ParseResult::Incomplete;
            }
            let s = match std::str::from_utf8(&self.rbuf[self.pos..self.pos + len]) {
                Ok(s) => s.to_owned(),
                Err(_) => return ParseResult::Error,
            };
            self.pos += len + 2;
            self.parts_buf.push(s);
        }

        ParseResult::Complete(&self.parts_buf)
    }

    #[inline(always)]
    fn scan_line(&mut self) -> Option<(usize, usize)> {
        let rel = memchr::memchr(b'\n', &self.rbuf[self.pos..self.filled])?;
        let start = self.pos;
        let nl = self.pos + rel;
        self.pos = nl + 1;
        let end = if nl > start && self.rbuf[nl - 1] == b'\r' { nl - 1 } else { nl };
        Some((start, end))
    }

    #[inline]
    pub fn write_response(&mut self, data: &[u8]) {
        self.wbuf.extend_from_slice(data);
    }

    #[inline]
    pub fn wbuf_mut(&mut self) -> &mut Vec<u8> {
        &mut self.wbuf
    }

    #[inline]
    pub fn take_wbuf(&mut self) -> &[u8] {
        &self.wbuf
    }

    #[inline]
    pub fn clear_wbuf(&mut self) {
        self.wbuf.clear();
    }

    #[inline]
    pub fn has_buffered_input(&self) -> bool {
        self.pos < self.filled
    }
}

#[inline(always)]
fn parse_usize(s: &[u8]) -> Option<usize> {
    if s.is_empty() { return None; }
    let mut n: usize = 0;
    for &b in s {
        if b < b'0' || b > b'9' { return None; }
        n = n.wrapping_mul(10).wrapping_add((b - b'0') as usize);
    }
    Some(n)
}
