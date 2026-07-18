use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::net::TcpStream;

pub struct RespParser {
    reader: tokio::net::tcp::OwnedReadHalf,
    writer: BufWriter<tokio::net::tcp::OwnedWriteHalf>,
    buf: Vec<u8>,
    filled: usize,
    pos: usize,
    parts_buf: Vec<String>,
}

impl RespParser {
    pub fn new(stream: TcpStream) -> Self {
        let (read_half, write_half) = stream.into_split();
        Self {
            reader: read_half,
            writer: BufWriter::with_capacity(65536, write_half),
            buf: vec![0u8; 65536],
            filled: 0,
            pos: 0,
            parts_buf: Vec::with_capacity(8),
        }
    }

    #[inline(always)]
    async fn fill(&mut self) -> io::Result<()> {
        if self.pos > 0 {
            self.buf.copy_within(self.pos..self.filled, 0);
            self.filled -= self.pos;
            self.pos = 0;
        }
        if self.filled == self.buf.len() {
            self.buf.resize(self.buf.len() * 2, 0);
        }
        let n = self.reader.read(&mut self.buf[self.filled..]).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed",
            ));
        }
        self.filled += n;
        Ok(())
    }

    #[inline(always)]
    async fn read_line_bytes(&mut self) -> io::Result<(usize, usize)> {
        loop {
            if let Some(rel) = memchr(b'\n', &self.buf[self.pos..self.filled]) {
                let start = self.pos;
                let nl = self.pos + rel;
                self.pos = nl + 1;
                let end = if nl > start && self.buf[nl - 1] == b'\r' {
                    nl - 1
                } else {
                    nl
                };
                return Ok((start, end));
            }
            self.fill().await?;
        }
    }

    #[inline(always)]
    async fn ensure_bytes(&mut self, n: usize) -> io::Result<()> {
        while self.filled - self.pos < n {
            self.fill().await?;
        }
        Ok(())
    }

    pub async fn parse(&mut self) -> io::Result<&[String]> {
        self.parts_buf.clear();

        let (s, e) = self.read_line_bytes().await?;
        if self.buf.get(s) != Some(&b'*') {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "expected array"));
        }
        let count = parse_usize(&self.buf[s + 1..e])
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid array length"))?;

        for _ in 0..count {
            let (bs, be) = self.read_line_bytes().await?;
            if self.buf.get(bs) != Some(&b'$') {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "expected bulk string",
                ));
            }
            let len = parse_usize(&self.buf[bs + 1..be]).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid bulk string length")
            })?;

            self.ensure_bytes(len + 2).await?;
            let s = std::str::from_utf8(&self.buf[self.pos..self.pos + len])
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid utf8"))?
                .to_owned();
            self.pos += len + 2;
            self.parts_buf.push(s);
        }

        Ok(&self.parts_buf)
    }

    #[inline]
    pub async fn write_response(&mut self, data: &[u8]) -> io::Result<()> {
        self.writer.write_all(data).await
    }

    #[inline]
    pub async fn flush(&mut self) -> io::Result<()> {
        self.writer.flush().await
    }

    #[inline]
    pub fn has_buffered_input(&self) -> bool {
        self.pos < self.filled
    }
}

#[inline(always)]
fn memchr(needle: u8, haystack: &[u8]) -> Option<usize> {
    haystack.iter().position(|&b| b == needle)
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
