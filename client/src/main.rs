use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};

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

    /// Port.
    #[arg(short, long, default_value_t = 6789)]
    port: u16,

    /// Timeout for read (in seconds).
    #[arg(short, long, default_value_t = 3)]
    timeout: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    SimpleLogger::new().init().unwrap();
    let args = Args::parse();

    log::info!("Making {} pings with {}ms gaps", args.count, args.gap);

    let mut join_handles = vec![];

    for _ in 0..args.count {
        let join_handle = spawn(async move {
            let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), args.port);
            let mut socket = TcpStream::connect(addr)
                .await
                .expect("Failed initiating connection");
            log::info!("Connecting to server");

            socket.write(b"hello").await.expect("Failed sending ping");
            log::info!("Sent ping");

            let mut buf: [u8; 32] = [0; 32];

            let timeout_res =
                time::timeout(Duration::from_secs(args.timeout), socket.read(&mut buf)).await;

            if timeout_res.is_err() {
                log::error!("Timeout error: {}", timeout_res.unwrap_err());
                return;
            }
            let read_res = timeout_res.unwrap();
            if read_res.is_err() {
                log::error!("Read error: {}", read_res.unwrap_err());
                return;
            }
            let resp_len = read_res.unwrap();

            log::info!(
                "Got back {} bytes: {}",
                resp_len,
                String::from_utf8_lossy(&mut buf)
            );
        });
        join_handles.push(join_handle);

        time::sleep(Duration::from_millis(args.gap)).await;
    }

    for join_handle in join_handles {
        join_handle.await.expect("Failed joining thread");
    }

    Ok(())
}
