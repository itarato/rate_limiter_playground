use std::time::Duration;

use simple_logger::SimpleLogger;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    SimpleLogger::new().init().unwrap();

    let mut socket = TcpStream::connect("127.0.0.1:6789").await?;
    log::info!("Connecting to server");

    socket.write(b"hello").await?;
    log::info!("Sent ping");

    let mut buf: [u8; 32] = [0; 32];

    time::timeout(Duration::from_secs(1), socket.read(&mut buf)).await??;
    log::info!("Got back: {}", String::from_utf8_lossy(&mut buf));

    Ok(())
}
