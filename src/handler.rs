use tokio::{io::AsyncWriteExt, net::TcpStream};

use crate::{
    parser::parse_resp,
    store::{delval, getval, setval},
};

pub async fn handle_client(mut socket: TcpStream) -> std::io::Result<()> {
    loop {
        let parts = match parse_resp(&mut socket).await {
            Ok(parts) => parts,
            Err(_) => break,
        };

        let response = match parts.as_slice() {
            [cmd] if cmd.eq_ignore_ascii_case("PING") => "+PONG\r\n".to_string(),

            [cmd, key, value] if cmd.eq_ignore_ascii_case("SET") => match key.parse::<i32>() {
                Ok(key) => {
                    setval(key, value.clone());
                    "+OK\r\n".to_string()
                }
                Err(_) => "-ERR invalid key\r\n".to_string(),
            },

            [cmd, key] if cmd.eq_ignore_ascii_case("GET") => match key.parse::<i32>() {
                Ok(key) => {
                    if let Some(value) = getval(key) {
                        format!("${}\r\n{}\r\n", value.len(), value)
                    } else {
                        "$-1\r\n".to_string()
                    }
                }
                Err(_) => "-ERR invalid key\r\n".to_string(),
            },

            [cmd, key] if cmd.eq_ignore_ascii_case("DEL") => match key.parse::<i32>() {
                Ok(key) => {
                    let removed = delval(key);
                    format!(":{}\r\n", if removed { 1 } else { 0 })
                }
                Err(_) => "-ERR invalid key\r\n".to_string(),
            },
            _ => "-ERR invalid command\r\n".to_string(),
        };
        socket.write_all(response.as_bytes()).await?;
    }
    Ok(())
}
