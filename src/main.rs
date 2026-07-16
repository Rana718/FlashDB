use std::time::Duration;

use flash_db::{handler::handle_client, store::DATAS};
use tokio::{net::TcpListener, time::Instant};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:8000").await?;

    println!("it runing on port 8000");

    tokio::task::spawn(async {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let now = Instant::now();
            DATAS.retain(|_, entry| {
               match entry.expires_at {
                   Some(exp) => exp > now,
                   None => true,
               }
            });
        }
    });

    loop {
        let (socket, addr) = listener.accept().await?;

        println!("user connected: {}", addr);

        tokio::spawn(async move {
            if let Err(err) = handle_client(socket).await {
                eprintln!("connection error: {}", err)
            }
        });
    }
}
