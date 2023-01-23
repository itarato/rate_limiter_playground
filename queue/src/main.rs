use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use simple_logger::SimpleLogger;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    spawn,
    sync::Mutex,
    time,
};

use clap::Parser;

type Error = Box<dyn std::error::Error + Send + Sync>;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Rate in seconds.
    #[arg(short, long, default_value_t = 4.0)]
    rate: f64,
}

enum ShouldThrottle {
    No,
    Yes(u64),
}

enum CanExit {
    No,
    Yes(u64),
}

struct RateLimiter {
    /// Poll key - assigned to pollers - to indicate their order.
    next_poll_key: u64,
    /// The poll ticket that can exit the queue.
    releasable_poll_key: u64,
    /// Last update time for `releasable_poll_key`.
    last_update: SystemTime,
    /// Poll exit rate: count per second.
    rate: f64,
    /// Timestamp on which the last rate reduction has been called.
    last_rate_reduce_second: u64,
}

impl RateLimiter {
    fn new(rate: f64) -> Self {
        Self {
            next_poll_key: 0,
            releasable_poll_key: 0,
            last_update: SystemTime::now(),
            rate,
            last_rate_reduce_second: UNIX_EPOCH.elapsed().expect("Cannot get epoch").as_secs(),
        }
    }

    fn should_throttle(&mut self) -> ShouldThrottle {
        self.refresh_poll_release_key();

        if self.next_poll_key < self.releasable_poll_key {
            self.next_poll_key = self.releasable_poll_key;
        }

        self.next_poll_key += 1;
        ShouldThrottle::Yes(self.next_poll_key - 1)
    }

    fn can_exit(&mut self, poll_key: u64) -> CanExit {
        self.refresh_poll_release_key();

        if poll_key <= self.releasable_poll_key {
            CanExit::Yes(0xdeadbeef)
        } else {
            CanExit::No
        }
    }

    fn refresh_poll_release_key(&mut self) {
        // Update releasable pointer.
        let elapsed_ms = self
            .last_update
            .elapsed()
            .expect("Failed calculating time")
            .as_millis() as f64;

        let increase = ((elapsed_ms / 1000.0) * self.rate) as u64;
        self.releasable_poll_key += increase;

        if increase > 0 {
            self.last_update = SystemTime::now();
        }
    }

    fn valid_request_key(&self, _request_key: u64) -> bool {
        // For now lets consider they are all legit.
        true
    }

    fn lower_rate(&mut self) {
        let current_timestamp = UNIX_EPOCH.elapsed().expect("Cannot get epoch").as_secs();

        if current_timestamp <= self.last_rate_reduce_second {
            log::info!("Reduction rejected - already done one in this second");
            return;
        }

        self.rate = 1.0f64.max(self.rate - 1.0);
        log::info!("Rate is lowered to: {}", self.rate);

        self.last_rate_reduce_second = current_timestamp;
    }
}

enum Request {
    UnauthorizedRequest(Vec<u8>),
    AuthorizedRequest(u64, Vec<u8>),
    Poll(u64),
}

async fn call_server(mut msg: Vec<u8>) -> Result<Vec<u8>, Error> {
    let mut socket = TcpStream::connect("127.0.0.1:5678")
        .await
        .map_err(|_| "Cannot connect to server")?;
    log::info!("Connecting to server");

    socket
        .write(&mut msg)
        .await
        .map_err(|_| "Cannot write to socket")?;
    log::info!("Sent msg");

    let mut buf: [u8; 32] = [0; 32];

    let read_len = time::timeout(Duration::from_secs(1), socket.read(&mut buf))
        .await
        .map_err(|_| "Timeout failed")?
        .map_err(|_| "Read from server failed")?;
    log::info!("Got back: {}", String::from_utf8_lossy(&mut buf));

    Ok(buf[..read_len].to_vec())
}

async fn read_request(stream: &mut TcpStream) -> Result<Request, Error> {
    let mut buf: [u8; 32] = [0; 32];
    let read_len = stream.read(&mut buf).await?;

    if read_len == b"POLL".len() + 8 && buf[8..12] == b"POLL"[..] {
        let poll_key = u64::from_be_bytes(buf[..8].try_into()?);
        Ok(Request::Poll(poll_key))
    } else if read_len >= 8 {
        let request_key = u64::from_be_bytes(buf[..8].try_into()?);
        if request_key > 0 {
            Ok(Request::AuthorizedRequest(request_key, buf[8..].to_vec()))
        } else {
            Ok(Request::UnauthorizedRequest(buf[8..].to_vec()))
        }
    } else {
        log::error!("Invalid request: {:?}", buf);
        Err("Invalid request".into())
    }
}

async fn handle_request(
    mut stream: TcpStream,
    rate_limiter: Arc<Mutex<RateLimiter>>,
) -> Result<(), Error> {
    let response = read_request(&mut stream).await?;

    {
        let rl = rate_limiter.lock().await;
        log::debug!(
            "RL >> rate: {} | next poll: {} | release: {}",
            rl.rate,
            rl.next_poll_key,
            rl.releasable_poll_key
        );
    }

    match response {
        Request::UnauthorizedRequest(msg) => {
            let should_throttle;
            {
                should_throttle = rate_limiter.lock().await.should_throttle();
            }

            match should_throttle {
                ShouldThrottle::Yes(poll_key) => {
                    let mut buf_out: [u8; 12] = [0; 12];
                    buf_out[..4].clone_from_slice(&b"POLL"[..]);
                    buf_out[4..12].clone_from_slice(&poll_key.to_be_bytes());

                    stream.write(&mut buf_out).await?;

                    log::info!("Client asked to poll with key: {}", poll_key);
                }
                ShouldThrottle::No => {
                    let mut response = call_server(msg).await?;
                    stream.write(&mut response).await?;

                    log::info!("Client can skip queue");
                }
            };

            Ok(())
        }
        Request::AuthorizedRequest(request_key, msg) => {
            if rate_limiter.lock().await.valid_request_key(request_key) {
                match call_server(msg).await {
                    Ok(mut response) => {
                        stream.write(&mut response).await?;
                        log::info!("Client was authorized to make request: {}", request_key);
                        Ok(())
                    }
                    Err(err) => {
                        rate_limiter.lock().await.lower_rate();
                        Err(err)
                    }
                }
            } else {
                unimplemented!()
            }
        }
        Request::Poll(poll_key) => {
            let can_exit;
            {
                can_exit = rate_limiter.lock().await.can_exit(poll_key);
            }

            match can_exit {
                CanExit::Yes(request_key) => {
                    let mut buf_out: [u8; 15] = [0; 15];
                    buf_out[..7].clone_from_slice(&b"REQUEST"[..]);
                    buf_out[7..15].clone_from_slice(&request_key.to_be_bytes());

                    stream.write(&mut buf_out).await?;

                    log::info!(
                        "Client can exit poll with key: {} -> request key {}",
                        poll_key,
                        request_key
                    );
                }
                CanExit::No => {
                    let mut buf_out: [u8; 4] = *b"POLL";

                    stream.write(&mut buf_out).await?;

                    log::info!("Client asked to keep polling for key: {}", poll_key);
                }
            };

            Ok(())
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    SimpleLogger::new().init().unwrap();
    let args = Args::parse();

    let listener = TcpListener::bind("0.0.0.0:6789").await?;
    log::info!("Listening on 6789");

    let rate_limiter = Arc::new(Mutex::new(RateLimiter::new(args.rate)));

    loop {
        let (stream, _addr) = listener.accept().await?;
        let _rate_limiter = rate_limiter.clone();

        spawn(async move {
            match handle_request(stream, _rate_limiter).await {
                Ok(_) => log::info!("Request handled"),
                Err(err) => log::error!("Request error: {}", err),
            }
        });
    }
}
