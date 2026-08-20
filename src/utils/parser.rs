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

const MAX_ARRAY_ELEMENTS: usize = 1024;
const MAX_BULK_BYTES: usize = 512 * 1024 * 1024;

impl Default for RespParser {
    fn default() -> Self {
        Self::new()
    }
}

impl RespParser {
    pub fn new() -> Self {
        Self {
            rbuf: vec![0u8; 2 * 1024],
            filled: 0,
            pos: 0,
            wbuf: Vec::with_capacity(1024),
            parts_raw: Vec::with_capacity(4),
        }
    }

    pub fn read_buf(&mut self) -> &mut [u8] {
        if self.rbuf.len() - self.filled < 1024 {
            if self.pos > 0 {
                self.rbuf.copy_within(self.pos..self.filled, 0);
                self.filled -= self.pos;
                self.pos = 0;
            }
            if self.rbuf.len() - self.filled < 1024 {
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
        let count = match parse_usize(&self.rbuf[s + 1..e], MAX_ARRAY_ELEMENTS) {
            Some(c) => c,
            None => return ParseResult::Error,
        };

        self.parts_raw.reserve(count);

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
            let len = match parse_usize(&self.rbuf[bs + 1..be], MAX_BULK_BYTES) {
                Some(l) => l,
                None => return ParseResult::Error,
            };

            if self.filled - self.pos < len + 2 {
                self.pos = start_pos;
                return ParseResult::Incomplete;
            }

            let ptr = self.rbuf[self.pos..].as_ptr();
            self.parts_raw.push((ptr, len));
            self.pos += len + 2;
        }

        // Shrink oversized read buffer after full drain
        if self.pos == self.filled && self.rbuf.len() > 16 * 1024 {
            self.rbuf.truncate(2 * 1024);
            self.rbuf.shrink_to(2 * 1024);
            self.filled = 0;
            self.pos = 0;
        }

        ParseResult::Complete
    }

    #[inline(always)]
    fn scan_line(&mut self) -> Option<(usize, usize)> {
        let rel = memchr::memchr(b'\n', &self.rbuf[self.pos..self.filled])?;
        let start = self.pos;
        let nl = self.pos + rel;
        self.pos = nl + 1;
        let end = if nl > start { nl - 1 } else { nl };
        Some((start, end))
    }
}

#[inline(always)]
fn parse_usize(s: &[u8], max: usize) -> Option<usize> {
    if s.is_empty() {
        return None;
    }
    if s.len() <= 3 {
        let mut n: usize = 0;
        for &b in s {
            if !b.is_ascii_digit() {
                return None;
            }
            n = n * 10 + (b - b'0') as usize;
        }
        if n > max {
            return None;
        }
        return Some(n);
    }
    let mut n: usize = 0;
    for &b in s {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as usize)?;
        if n > max {
            return None;
        }
    }
    Some(n)
}
