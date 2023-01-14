use std::{collections::VecDeque, sync::Arc, time::Duration};

use simple_logger::SimpleLogger;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    spawn,
    sync::Mutex,
    time,
};

struct LeakyBucket {
    limit: usize,
    rate: f64, // Per seconds.
    queue: VecDeque<TcpStream>,
}

impl LeakyBucket {
    fn new(limit: usize, rate: f64) -> Self {
        Self {
            limit,
            rate,
            queue: VecDeque::new(),
        }
    }

    fn accept(&mut self, stream: TcpStream) -> Option<TcpStream> {
        if self.queue.len() >= self.limit {
            Some(stream)
        } else {
            self.queue.push_back(stream);
            None
        }
    }

    async fn leak(&mut self) {
        let stream = self.queue.pop_front();
        if stream.is_none() {
            return;
        }
        let mut stream = stream.unwrap();

        let mut buf: [u8; 32] = [0; 32];

        match stream.read(&mut buf).await {
            Ok(len) => {
                log::info!("Incoming msg: {}", String::from_utf8_lossy(&mut buf));

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

    let leaky_bucket = Arc::new(Mutex::new(LeakyBucket::new(4, 2.0)));
    let bucket_release = leaky_bucket.clone();

    spawn(async move {
        loop {
            let bucket_rate: f64;
            {
                let mut bucket = bucket_release.lock().await;
                bucket.leak().await;
                bucket_rate = bucket.rate;
            }

            let milli_rate = (1000.0 / bucket_rate) as u64;
            time::sleep(Duration::from_millis(milli_rate)).await;
        }
    });

    loop {
        let (stream, addr) = listener.accept().await?;
        log::info!("Incoming connection from: {}", addr);

        let accept_res: Option<TcpStream>;
        {
            accept_res = leaky_bucket.lock().await.accept(stream);
        }

        match accept_res {
            Some(mut stream) => {
                match stream.write(b"no").await {
                    Ok(_) => log::info!("Ping deny"),
                    Err(err) => log::error!("Ping deny error: {}", err),
                };
            }
            None => {
                log::info!("Stream is queued");
            }
        };
    }
}
