use simple_logger::SimpleLogger;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    spawn,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    SimpleLogger::new().init().unwrap();

    let listener = TcpListener::bind("0.0.0.0:5678").await?;
    log::info!("Listening on 5678");

    loop {
        let (mut stream, addr) = listener.accept().await?;
        log::info!("Incoming connection from: {}", addr);

        spawn(async move {
            let mut buf: [u8; 32] = [0; 32];

            match stream.read(&mut buf).await {
                Ok(len) => {
                    log::info!("Incoming msg: {}", String::from_utf8_lossy(&mut buf));
                    match stream.write(&mut buf[..len]).await {
                        Ok(_) => log::info!("Ping returned"),
                        Err(err) => log::error!("Ping write error: {}", err),
                    }
                }
                Err(err) => {
                    log::error!("Failed reading from socket: {}", err);
                }
            }
        });
    }
}
