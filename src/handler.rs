use crate::{commends, storage::store::Store};
use crate::parser::parse_resp;
use tokio::{io::AsyncWriteExt, net::TcpStream};
use std::sync::Arc;

pub async fn handle_client(mut socket: TcpStream, store: Arc<Store>) -> std::io::Result<()> {
    loop {
        let parts = match parse_resp(&mut socket).await {
            Ok(parts) => parts,
            Err(_) => break,
        };
        let response = commends::execute(parts, &store).await;
        socket.write_all(response.as_bytes()).await?;
    }
    Ok(())
}
