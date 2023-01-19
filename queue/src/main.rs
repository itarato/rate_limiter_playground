use std::{error::Error, sync::Arc, time::Duration};

use simple_logger::SimpleLogger;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    spawn,
    sync::Mutex,
    time,
};

struct RateLimiter {}

impl RateLimiter {
    fn new() -> Self {
        Self {}
    }

    fn should_throttle(&self) -> bool {
        false
    }
}

enum Request {
    UnauthorizedRequest(Vec<u8>),
    AuthorizedRequest(u64, Vec<u8>),
    Poll(u64),
}

async fn call_server(mut msg: Vec<u8>) -> Result<(Vec<u8>), Box<dyn Error>> {
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

async fn read_request(stream: &mut TcpStream) -> Result<Request, Box<dyn Error>> {
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
) -> Result<(), Box<dyn Error>> {
    let response = read_request(&mut stream).await?;

    match response {
        Request::UnauthorizedRequest(msg) => {
            // Can be served?
            // - yes -> call_server(...)
            // - no  -> send back poll with key

            let should_throttle: bool;
            {
                should_throttle = rate_limiter.lock().await.should_throttle();
            }

            if should_throttle {
                unimplemented!()
            } else {
                let mut response = call_server(msg).await?;
                stream.write(&mut response).await?;

                Ok(())
            }
        }
        Request::AuthorizedRequest(request_key, msg) => {
            // Key valid?
            // - yes -> call_server(...)
            // - no  -> send back poll with key

            unimplemented!()
        }
        Request::Poll(poll_key) => {
            // Poll key ready to release?
            // - yes -> provision request key
            // - no  -> keep polling message

            unimplemented!()
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    SimpleLogger::new().init().unwrap();

    let listener = TcpListener::bind("0.0.0.0:6789").await?;
    log::info!("Listening on 6789");

    let rate_limiter = Arc::new(Mutex::new(RateLimiter::new()));

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
