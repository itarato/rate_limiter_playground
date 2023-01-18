use std::{error::Error, time::Duration};

use simple_logger::SimpleLogger;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    spawn, time,
};

use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Number of calls.
    #[arg(short, long, default_value_t = 1)]
    count: usize,

    /// Gap between requests (in milliseconds).
    #[arg(short, long, default_value_t = 1)]
    gap: u64,
}

#[derive(Debug)]
enum Response {
    Pong(Vec<u8>), // Actual server response.
    StartPoll(u64),
    KeepPolling,
    RequestGranted(u64),
}

enum ClientState {
    Init,
    Polling(u64),
    Authorized(u64),
    Completed,
    Failed,
}

struct PollingClient {
    state: ClientState,
}

impl PollingClient {
    fn new() -> Self {
        PollingClient {
            state: ClientState::Init,
        }
    }

    async fn attempt(&mut self) {
        let mut stream = TcpStream::connect("127.0.0.1:6789")
            .await
            .expect("Failed initiating connection");
        log::info!("Connecting to server");

        match self.state {
            ClientState::Init => {
                stream
                    .write(b"\0\0\0\0\0\0\0\0hello")
                    .await
                    .expect("Failed sending ping");
                log::info!("Sent ping");

                match Self::read_response(&mut stream).await {
                    Ok(Response::Pong(msg)) => {
                        self.state = ClientState::Completed;
                        log::info!(
                            "Got response: {}",
                            String::from_utf8(msg).expect("Failed parsing pong message")
                        );
                    }
                    Ok(Response::StartPoll(poll_key)) => {
                        self.state = ClientState::Polling(poll_key);
                        log::info!("Throttled, start polling.");
                    }
                    Ok(resp) => unreachable!("Incorrect response: {:?}", resp),
                    Err(err) => {
                        log::error!("Failed request: {}", err);
                        self.state = ClientState::Failed;
                    }
                }
            }
            ClientState::Polling(poll_key) => {
                let mut write_buf: [u8; 12] = [0; 12];
                write_buf[..8].clone_from_slice(&poll_key.to_be_bytes());
                write_buf[8..12].clone_from_slice(&b"POLL"[..]);

                stream
                    .write(&mut write_buf)
                    .await
                    .expect("Failed sending ping");

                match Self::read_response(&mut stream).await {
                    Ok(Response::KeepPolling) => {
                        log::info!("Poll result: keep polling");
                    }
                    Ok(Response::RequestGranted(request_key)) => {
                        self.state = ClientState::Authorized(request_key);
                        log::info!("Poll result: request granted");
                    }
                    Ok(resp) => unreachable!("Incorrect response: {:?}", resp),
                    Err(err) => {
                        log::error!("Failed request: {}", err);
                        self.state = ClientState::Failed;
                    }
                }
            }
            ClientState::Authorized(request_key) => {
                let mut write_buf: [u8; 13] = [0; 13];
                write_buf[..8].clone_from_slice(&request_key.to_be_bytes());
                write_buf[8..13].clone_from_slice(&b"hello"[..]); // Repeat ping message.

                stream
                    .write(&mut write_buf)
                    .await
                    .expect("Failed sending ping");

                match Self::read_response(&mut stream).await {
                    Ok(Response::Pong(msg)) => {
                        self.state = ClientState::Completed;
                        log::info!(
                            "Got pong response: {}",
                            String::from_utf8(msg).expect("Failed parsing pong response")
                        );
                    }
                    Ok(resp) => unreachable!("Incorrect response: {:?}", resp),
                    Err(err) => {
                        log::error!("Failed request: {}", err);
                        self.state = ClientState::Failed;
                    }
                }
            }
            ClientState::Completed => unreachable!("Completed client should not do attempts"),
            ClientState::Failed => unreachable!("Failed client should not do attempts"),
        }
    }

    async fn read_response(stream: &mut TcpStream) -> Result<Response, Box<dyn Error>> {
        let mut buf: [u8; 32] = [0; 32];
        let resp_len = time::timeout(Duration::from_secs(3), stream.read(&mut buf)).await??;

        log::info!(
            "Got back {} bytes: {}",
            resp_len,
            String::from_utf8_lossy(&mut buf)
        );

        if resp_len == b"POLL".len() && buf[..4] == b"POLL"[..] {
            Ok(Response::KeepPolling)
        } else if resp_len == b"POLL".len() + 8 && buf[..4] == b"POLL"[..] {
            let request_key = u64::from_be_bytes(
                buf[4..12]
                    .try_into()
                    .expect("Failed extracting request key"),
            );
            Ok(Response::StartPoll(request_key))
        } else if resp_len == b"REQUEST".len() + 8 && buf[..7] == b"REQUEST"[..] {
            let request_key = u64::from_be_bytes(
                buf[7..15]
                    .try_into()
                    .expect("Failed extracting request key"),
            );
            Ok(Response::RequestGranted(request_key))
        } else {
            Ok(Response::Pong(buf[..resp_len].to_vec()))
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    SimpleLogger::new().init().unwrap();
    let args = Args::parse();

    log::info!("Making {} pings with {}ms gaps", args.count, args.gap);

    let mut join_handles = vec![];

    for _ in 0..args.count {
        let join_handle = spawn(async move {
            let mut client = PollingClient::new();

            loop {
                client.attempt().await;

                match client.state {
                    ClientState::Failed => {
                        log::warn!("Client has failed");
                        break;
                    }
                    ClientState::Completed => {
                        log::info!("Client completed");
                        break;
                    }
                    _ => time::sleep(Duration::from_secs(1)).await,
                }
            }
        });
        join_handles.push(join_handle);

        time::sleep(Duration::from_millis(args.gap)).await;
    }

    for join_handle in join_handles {
        join_handle.await.expect("Failed joining thread");
    }

    Ok(())
}
