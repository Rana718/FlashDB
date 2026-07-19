pub struct RespParser {
    pub rbuf: Vec<u8>,
    pub filled: usize,
    pub pos: usize,
    pub wbuf: Vec<u8>,
    pub parts_raw: Vec<(*const u8, usize)>,
}

unsafe impl Send for RespParser {}

pub enum ParseResult {
    Complete,
    Incomplete,
    Error,
}

impl RespParser {
    pub fn new() -> Self {
        Self {
            rbuf: vec![0u8; 65536],
            filled: 0,
            pos: 0,
            wbuf: Vec::with_capacity(256 * 1024),
            parts_raw: Vec::with_capacity(8),
        }
    }

    #[inline]
    pub fn parts(&self) -> impl Iterator<Item = &str> {
        self.parts_raw.iter().map(|&(ptr, len)| unsafe {
            let slice = std::slice::from_raw_parts(ptr, len);
            std::str::from_utf8_unchecked(slice)
        })
    }

    #[inline]
    pub fn parts_len(&self) -> usize {
        self.parts_raw.len()
    }

    #[inline]
    pub fn part(&self, i: usize) -> &str {
        let (ptr, len) = self.parts_raw[i];
        unsafe {
            let slice = std::slice::from_raw_parts(ptr, len);
            std::str::from_utf8_unchecked(slice)
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

    pub fn parse_one(&mut self) -> ParseResult {
        self.parts_raw.clear();

        let start_pos = self.pos;

        let (s, e) = match self.scan_line() {
            Some(r) => r,
            None => {
                self.pos = start_pos;
                return ParseResult::Incomplete;
            }
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
                None => {
                    self.pos = start_pos;
                    return ParseResult::Incomplete;
                }
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
            if std::str::from_utf8(&self.rbuf[self.pos..self.pos + len]).is_err() {
                return ParseResult::Error;
            }
            let ptr = self.rbuf[self.pos..].as_ptr();
            self.parts_raw.push((ptr, len));
            self.pos += len + 2;
        }

        ParseResult::Complete
    }

    #[inline(always)]
    fn scan_line(&mut self) -> Option<(usize, usize)> {
        let rel = memchr::memchr(b'\n', &self.rbuf[self.pos..self.filled])?;
        let start = self.pos;
        let nl = self.pos + rel;
        self.pos = nl + 1;
        let end = if nl > start && self.rbuf[nl - 1] == b'\r' {
            nl - 1
        } else {
            nl
        };
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
    if s.is_empty() {
        return None;
    }
    let mut n: usize = 0;
    for &b in s {
        if b < b'0' || b > b'9' {
            return None;
        }
        n = n.wrapping_mul(10).wrapping_add((b - b'0') as usize);
    }
    Some(n)
}
