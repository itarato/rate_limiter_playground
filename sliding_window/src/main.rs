use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, SystemTime},
};

use simple_logger::SimpleLogger;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    spawn,
    sync::Mutex,
    time,
};

struct SlidingWindow {
    limit: usize, // Per second.
    log: VecDeque<SystemTime>,
}

impl SlidingWindow {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            log: VecDeque::new(),
        }
    }

    fn is_allow(&mut self) -> bool {
        // Reset.
        let cutoff = SystemTime::now() - Duration::from_secs(1);
        loop {
            match self.log.front() {
                Some(t) => {
                    if t <= &cutoff {
                        self.log.pop_front().expect("Failed popping front.");
                    } else {
                        break;
                    }
                }
                None => break,
            };
        }

        if self.log.len() < self.limit {
            self.log.push_back(SystemTime::now());
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

    let sliding_window = Arc::new(Mutex::new(SlidingWindow::new(2)));

    loop {
        let (mut stream, addr) = listener.accept().await?;
        log::info!("Incoming connection from: {}", addr);

        let _sliding_window = sliding_window.clone();

        spawn(async move {
            let mut buf: [u8; 32] = [0; 32];

            match stream.read(&mut buf).await {
                Ok(len) => {
                    log::info!("Incoming msg: {}", String::from_utf8_lossy(&mut buf));

                    if !_sliding_window.lock().await.is_allow() {
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
