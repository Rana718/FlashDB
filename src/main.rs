use flash_db::handler::handle_client;
use tokio::{
    net::TcpListener,
};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:8000").await?;

    println!("it runing on port 8000");

    loop {
       let (socket, addr) = listener.accept().await?;

       println!("user connected: {}", addr);

       tokio::spawn(async move {
           if let Err(err) = handle_client(socket).await{
              eprintln!("connection error: {}", err)
           }
       });
    }

   
}
