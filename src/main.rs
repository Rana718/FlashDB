use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use dashmap::DashMap;
use std::{
    sync::{LazyLock, Mutex},
};

static USERS: LazyLock<Mutex<DashMap<i32, String>>> = LazyLock::new(|| Mutex::new(DashMap::new()));

fn setval(key: i32, value: String) {
    USERS.lock().unwrap().insert(key, value);
}

fn getval(key: i32) -> Option<String> {
   USERS.lock().unwrap().get(&key).map(|v| v.value().to_string())
}

#[tokio::main]
async fn main() {
    let listener = TcpListener::bind("127.0.0.1:8000").await.unwrap();

    loop {
        let (mut socket, _) = listener.accept().await.unwrap();

        tokio::spawn(async move {
            let mut buf = [0u8; 1024];

            loop {
                let n = socket.read(&mut buf).await.unwrap();

                let slice: &[u8] = &buf[0..n];
                let cmd = String::from_utf8_lossy(slice).trim().to_string();

                let response = {
                    let parts: Vec<&str> = cmd.split_whitespace().collect();

                    match parts.as_slice() {
                        ["add", key, value] => match key.parse::<i32>() {
                            Ok(key) => {
                                setval(key, value.to_string());
                                "ok\n".to_string()
                            }
                            Err(err) => {
                                eprintln!("[ERROR] Invalid key {}", err);
                                "invalid key\n".to_string()
                            }
                        },
                        ["get", key] => match key.parse::<i32>() {
                            Ok(key) => getval(key)
                                .map(|value| format!("ok: {}\n", value))
                                .unwrap_or_else(|| "error: key not found\n".to_string()),

                            Err(err) => {
                                eprintln!("[ERROR] Invalid key '{}': {}", key, err);
                                "error: invalid key\n".to_string()
                            }
                        },
                        ["ping"] => "pong\n".to_string(),
                        _ => "unknown command\n".to_string(),
                    }
                };
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
    }
}
