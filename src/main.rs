use flash_db::{handler::handle_client, storage::store::Store};
use std::{sync::Arc, time::Duration};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let listener = TcpListener::bind("0.0.0.0:8000").await?;

    let store = Arc::new(Store::new());
    let store_clone = Arc::clone(&store);

    println!("flashdb running on port 8000");

    tokio::task::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            store_clone.cleanup_expired();
        }
    });

    loop {
        let (socket, _addr) = listener.accept().await?;
        socket.set_nodelay(true)?;
        let store = Arc::clone(&store);

        tokio::spawn(async move {
            if let Err(err) = handle_client(socket, store).await {
                if err.kind() != std::io::ErrorKind::UnexpectedEof
                    && err.kind() != std::io::ErrorKind::ConnectionReset
                {
                    eprintln!("connection error: {}", err);
                }
            }
        });
    }
}
