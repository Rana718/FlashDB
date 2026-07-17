use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::TcpStream;

pub struct RespParser {
    reader: BufReader<tokio::net::tcp::OwnedReadHalf>,
    writer: BufWriter<tokio::net::tcp::OwnedWriteHalf>,
}

impl RespParser {
    pub fn new(stream: TcpStream) -> Self {
        let (read_half, write_half) = stream.into_split();
        Self {
            // 32KB read buffer
            reader: BufReader::with_capacity(32768, read_half),
            // 32KB write buffer
            writer: BufWriter::with_capacity(32768, write_half),
        }
    }

    pub async fn parse(&mut self) -> std::io::Result<Vec<String>> {
        let mut line = String::new();

        let n = self.reader.read_line(&mut line).await?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "connection closed",
            ));
        }

        let trimmed = line.trim_end();
        if !trimmed.starts_with('*') {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "expected array",
            ));
        }

        let count: usize = trimmed[1..].parse().map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid array length")
        })?;

        let mut result = Vec::with_capacity(count);

        for _ in 0..count {
            line.clear();
            self.reader.read_line(&mut line).await?;
            let header = line.trim_end();

            if !header.starts_with('$') {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "expected bulk string",
                ));
            }

            let len: usize = header[1..].parse().map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid bulk string length")
            })?;

            let mut data = vec![0u8; len + 2];
            self.reader.read_exact(&mut data).await?;
            data.truncate(len);

            let s = String::from_utf8(data).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid utf8")
            })?;
            result.push(s);
        }

        Ok(result)
    }

    #[inline]
    pub async fn write_response(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(data).await
    }

    #[inline]
    pub async fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush().await
    }

    #[inline]
    pub fn has_buffered_input(&self) -> bool {
        !self.reader.buffer().is_empty()
    }
}
