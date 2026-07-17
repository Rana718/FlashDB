use crate::{commends, storage::store::Store, utils::parser::RespParser};
use std::sync::Arc;
use tokio::net::TcpStream;

pub async fn handle_client(socket: TcpStream, store: Arc<Store>) -> std::io::Result<()> {
    store.client_connected();

    let mut parser = RespParser::new(socket);

    loop {
        let parts = match parser.parse().await {
            Ok(parts) => parts,
            Err(_) => break,
        };

        let response = commends::execute(parts, &store).await;
        parser.write_response(response.as_bytes()).await?;

        // Only flush when the read buffer is empty — i.e. no more pipelined
        // commands are waiting. This batches responses for pipelined clients
        // while staying responsive for sequential clients.
        if !parser.has_buffered_input() {
            parser.flush().await?;
        }
    }

    // Flush any remaining buffered responses before closing
    let _ = parser.flush().await;

    store.client_disconnected();
    Ok(())
}
