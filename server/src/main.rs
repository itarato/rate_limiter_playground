use std::{collections::VecDeque, sync::Arc, time::Duration};

use simple_logger::SimpleLogger;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    spawn,
    sync::Mutex,
};

use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Number of workers.
    #[arg(short, long, default_value_t = 1)]
    worker: usize,

    /// Latency of a response (in milliseconds).
    #[arg(short, long, default_value_t = 0)]
    delay: u64,
}

async fn broadcast_backlog(job_queue: Arc<Mutex<VecDeque<TcpStream>>>) {
    let listener = TcpListener::bind("0.0.0.0:4567")
        .await
        .expect("Failed listning for broadcast");

    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("Failed accepting backlog broadcaster connection");

        let backlog: u64;
        {
            backlog = job_queue.lock().await.len() as u64;
        }

        stream
            .write(&mut backlog.to_be_bytes())
            .await
            .expect("Failed broadcasting");
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    SimpleLogger::new().init().unwrap();

    let args = Args::parse();

    let listener = TcpListener::bind("0.0.0.0:5678").await?;
    log::info!("Listening on 5678");

    let job_queue: Arc<Mutex<VecDeque<TcpStream>>> = Arc::new(Mutex::new(VecDeque::new()));

    let mut _job_queue = job_queue.clone();
    spawn(async move {
        broadcast_backlog(_job_queue).await;
    });

    for _ in 0..args.worker {
        let _job_queue = job_queue.clone();
        let _delay = args.delay;

        spawn(async move {
            loop {
                let stream: Option<TcpStream>;
                {
                    let mut jobs = _job_queue.lock().await;
                    stream = jobs.pop_front();
                }

                if stream.is_none() {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }

                let mut stream = stream.unwrap();
                let mut buf: [u8; 32] = [0; 32];
                match stream.read(&mut buf).await {
                    Ok(len) => {
                        log::info!("Incoming msg: {}", String::from_utf8_lossy(&mut buf));

                        tokio::time::sleep(Duration::from_millis(_delay)).await;

                        match stream.write(&mut buf[..len]).await {
                            Ok(_) => log::info!("Ping returned"),
                            Err(err) => log::error!("Ping write error: {}", err),
                        };
                    }
                    Err(err) => log::error!("Failed reading from stream: {}", err),
                };
            }
        });
    }

    loop {
        let (stream, addr) = listener.accept().await?;
        log::info!("Incoming connection from: {}", addr);

        {
            job_queue.lock().await.push_back(stream);
        }
    }
}
