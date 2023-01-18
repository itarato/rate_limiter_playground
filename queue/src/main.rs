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

async fn call_server(mut msg: Vec<u8>) -> Result<(usize, [u8; 32]), Box<dyn Error>> {
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

    Ok((read_len, buf))
}

async fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buf: [u8; 32] = [0; 32];
    let read_res = stream.read(&mut buf).await;

    if read_res.is_err() {
        log::error!("Read error: {}", read_res.unwrap_err());
        return None;
    }
    let read_len = read_res.unwrap();

    if read_len == b"POLL".len() + 8 && buf[8..12] == b"POLL"[..] {
        let poll_key = u64::from_be_bytes(buf[..8].try_into().expect("Cannot read poll key"));
        Some(Request::Poll(poll_key))
    } else if read_len >= 8 {
        let request_key = u64::from_be_bytes(buf[..8].try_into().expect("Cannot read request key"));
        if request_key > 0 {
            Some(Request::AuthorizedRequest(request_key, buf[8..].to_vec()))
        } else {
            Some(Request::UnauthorizedRequest(buf[8..].to_vec()))
        }
    } else {
        log::error!("Invalid request: {:?}", buf);
        None
    }
}

async fn handle_request(mut stream: TcpStream, rate_limiter: Arc<Mutex<RateLimiter>>) {
    match call_server(vec![]).await {
        Ok(_) => {}
        Err(a) => log::error!("{}", a),
    };

    // match read_request(&mut stream).await {
    //     Some(Request::UnauthorizedRequest(mut msg)) => {
    //         // Can be served?
    //         // - yes -> call_server(...)
    //         // - no  -> send back poll with key

    //         let should_throttle: bool;
    //         {
    //             should_throttle = rate_limiter.lock().await.should_throttle();
    //         }

    //         if should_throttle {
    //             unimplemented!()
    //         } else {
    //             match call_server(msg).await {
    //                 Ok(_) => {}
    //                 Err(a) => log::error!("{}", a),
    //             };
    //         }

    //         unimplemented!()
    //     }
    //     Some(Request::AuthorizedRequest(request_key, msg)) => {
    //         // Key valid?
    //         // - yes -> call_server(...)
    //         // - no  -> send back poll with key

    //         unimplemented!()
    //     }
    //     Some(Request::Poll(poll_key)) => {
    //         // Poll key ready to release?
    //         // - yes -> provision request key
    //         // - no  -> keep polling message

    //         unimplemented!()
    //     }
    //     None => log::error!("Request handle error"),
    // };
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
            handle_request(stream, _rate_limiter).await;
        });
    }
}
