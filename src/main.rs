use flash_db::{handler::handle_client, storage::store::Store};
use std::{sync::Arc, time::Duration};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:8000").await?;

    let store = Arc::new(Store::new());
    let store_clone = Arc::clone(&store);

    println!("it runing on port 8000");

    tokio::task::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            store_clone.cleanup_expired();
        }
    });

    loop {
        let (socket, addr) = listener.accept().await?;
        let store = Arc::clone(&store);
        println!("user connected: {}", addr);

        tokio::spawn(async move {
            if let Err(err) = handle_client(socket, store).await {
                eprintln!("connection error: {}", err)
            }
        });
    }
}
