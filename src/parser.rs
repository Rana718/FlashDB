use tokio::io::AsyncReadExt;

pub async fn parse_resp(socket: &mut tokio::net::TcpStream) -> std::io::Result<Vec<String>> {
    let mut first = [0u8; 1];
    socket.read_exact(&mut first).await?;

    if first[0] != b'*' {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "expected array",
        ));
    }

    let count = read_number(socket).await?;

    let mut result = Vec::new();

    for _ in 0..count {
        socket.read_exact(&mut first).await?;

        if first[0] != b'$' {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "expected bulk string",
            ));
        }

        let len = read_number(socket).await? as usize;

        let mut data = vec![0u8; len];
        socket.read_exact(&mut data).await?;

        let mut crlf = [0u8; 2];
        socket.read_exact(&mut crlf).await?;

        result.push(String::from_utf8(data).unwrap());
    }

    Ok(result)
}

async fn read_number(socket: &mut tokio::net::TcpStream) -> std::io::Result<i64> {
    let mut buf = Vec::new();

    loop {
        let mut byte = [0u8; 1];
        socket.read_exact(&mut byte).await?;

        if byte[0] == b'\r' {
            let mut lf = [0u8; 1];
            socket.read_exact(&mut lf).await?;
            break;
        }

        buf.push(byte[0]);
    }

    Ok(String::from_utf8(buf).unwrap().parse().unwrap())
}
