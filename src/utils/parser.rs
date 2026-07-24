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
    pub fn parts_as_raw(&self) -> smallvec::SmallVec<[(*const u8, usize); 32]> {
        self.parts_raw.iter().copied().collect()
    }

    #[inline]
    pub fn parts_as_strs(&self) -> smallvec::SmallVec<[&str; 32]> {
        self.parts_raw
            .iter()
            .map(|&(ptr, len)| unsafe {
                std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len))
            })
            .collect()
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
        if self.rbuf.len() - self.filled < 4096 {
            if self.pos > 0 {
                self.rbuf.copy_within(self.pos..self.filled, 0);
                self.filled -= self.pos;
                self.pos = 0;
            }
            if self.rbuf.len() - self.filled < 4096 {
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

    #[inline]
    pub fn has_buffered_input(&self) -> bool {
        self.pos < self.filled
    }
}

#[inline(always)]
fn parse_usize(s: &[u8], max: usize) -> Option<usize> {
    if s.is_empty() {
        return None;
    }
    let mut n: usize = 0;
    for &b in s {
        if b < b'0' || b > b'9' {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as usize)?;
        if n > max {
            return None;
        }
    }
    Some(n)
}
