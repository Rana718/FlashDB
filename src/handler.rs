use std::time::Duration;

use tokio::time::Instant;
use tokio::{io::AsyncWriteExt, net::TcpStream};

use crate::store::StoreValue;

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

            [cmd, key, value, rest @ ..] if cmd.eq_ignore_ascii_case("SET") => {
                let expires_at = match rest {
                    [] => None,

                    [ttl_cmd, sec] if ttl_cmd.eq_ignore_ascii_case("EX") => sec
                        .parse::<u64>()
                        .ok()
                        .map(|s| Instant::now() + Duration::from_secs(s)),
                    _ => {
                        return socket.write_all(b"-ERR invaild arguments\r\n").await;
                    }
                };

                match key.parse::<i32>() {
                    Ok(key) => {
                        setval(
                            key,
                            StoreValue {
                                value: value.clone(),
                                expires_at,
                            },
                        );
                        "+OK\r\n".to_string()
                    }
                    Err(_) => "-ERR invalid key\r\n".to_string(),
                }
            }

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
