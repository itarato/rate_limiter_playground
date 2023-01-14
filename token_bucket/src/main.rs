use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};

use log::info;
use simple_logger::SimpleLogger;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    spawn,
    sync::Mutex,
    time,
};

struct RateLimiter {
    bucket: u64,
    limit: u64,
    rate: f64, // Per second.
    last_update: SystemTime,
}

impl RateLimiter {
    fn new(limit: u64, rate: f64) -> Self {
        Self {
            bucket: limit,
            limit,
            rate,
            last_update: std::time::SystemTime::now(),
        }
    }

    fn is_allow(&mut self, req: u64) -> bool {
        // Refill.
        let elapsed = self
            .last_update
            .elapsed()
            .expect("Failed calculating elapsed time")
            .as_millis();
        self.last_update = SystemTime::now();

        let new_tokens = (((elapsed as f64) / 1000.0) * self.rate) as u64;
        let new_cap = new_tokens.min(self.limit - self.bucket);

        info!(
            "Bucket refilled {} -> {}",
            self.bucket,
            self.bucket + new_cap
        );
        self.bucket += new_cap;

        if self.bucket >= req {
            self.bucket -= req;
            true
        } else {
            false
        }
    }
}

async fn proxy(len: usize, msg: &[u8]) -> Result<(usize, [u8; 32]), &str> {
    let mut socket = TcpStream::connect("127.0.0.1:5678")
        .await
        .map_err(|_| "Cannot connect to server")?;
    log::info!("Connecting to server");

    socket
        .write(&msg[..len])
        .await
        .map_err(|_| "Cannot write to socket")?;
    log::info!("Sent msg");

    let mut buf: [u8; 32] = [0; 32];

    let read_len = time::timeout(Duration::from_secs(1), socket.read(&mut buf))
        .await
        .map_err(|_| "Timeout failed")?
        .map_err(|_| "Read from server failed")?;
    log::info!("Got back: {}", String::from_utf8_lossy(&mut buf));

    Ok((read_len, buf))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    SimpleLogger::new().init().unwrap();

    let listener = TcpListener::bind("0.0.0.0:6789").await?;
    log::info!("Listening on 6789");

    let rate_limiter = Arc::new(Mutex::new(RateLimiter::new(10, 5.0)));

    loop {
        let (mut stream, addr) = listener.accept().await?;
        log::info!("Incoming connection from: {}", addr);

        let _rate_limiter = rate_limiter.clone();

        spawn(async move {
            let mut buf: [u8; 32] = [0; 32];

            match stream.read(&mut buf).await {
                Ok(len) => {
                    log::info!("Incoming msg: {}", String::from_utf8_lossy(&mut buf));

                    if !_rate_limiter.lock().await.is_allow(len as u64) {
                        log::warn!("Request throttled");

                        match stream.write(b"no").await {
                            Ok(_) => log::info!("Ping deny"),
                            Err(err) => log::error!("Ping deny error: {}", err),
                        };

                        return;
                    }

                    match proxy(len, &buf).await {
                        Ok((len, mut resp)) => {
                            match stream.write(&mut resp[..len]).await {
                                Ok(_) => log::info!("Ping returned"),
                                Err(err) => log::error!("Ping write error: {}", err),
                            };
                        }
                        Err(err) => log::error!("Failed server comm: {}", err),
                    };
                }
                Err(err) => {
                    log::error!("Failed reading from socket: {}", err);
                }
            };
        });
    }
}
