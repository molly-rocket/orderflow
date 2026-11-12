use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

#[derive(Debug)]
struct Shared {
    responses: Vec<(u16, String)>,
    served: AtomicUsize,
    paths: Mutex<Vec<String>>,
}

#[derive(Debug)]
pub struct FakeRest {
    addr: SocketAddr,
    shared: Arc<Shared>,
    accept_loop: JoinHandle<()>,
}

impl FakeRest {
    pub async fn start(responses: Vec<(u16, String)>) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared {
            responses,
            served: AtomicUsize::new(0),
            paths: Mutex::default(),
        });
        let accept_loop = tokio::spawn(accept_loop(listener, Arc::clone(&shared)));
        Ok(Self {
            addr,
            shared,
            accept_loop,
        })
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn requested_paths(&self) -> Vec<String> {
        self.shared
            .paths
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Drop for FakeRest {
    fn drop(&mut self) {
        self.accept_loop.abort();
    }
}

async fn accept_loop(listener: TcpListener, shared: Arc<Shared>) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        tokio::spawn(serve(stream, Arc::clone(&shared)));
    }
}

async fn serve(mut stream: TcpStream, shared: Arc<Shared>) {
    let mut request = Vec::new();
    let mut chunk = [0u8; 1024];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => request.extend_from_slice(&chunk[..n]),
        }
    }
    let head = String::from_utf8_lossy(&request);
    let path = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_owned();
    shared
        .paths
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(path);
    let index = shared.served.fetch_add(1, Ordering::SeqCst);
    let (status, body) = shared
        .responses
        .get(index)
        .or(shared.responses.last())
        .cloned()
        .unwrap_or((404, String::new()));
    if status == 0 {
        let mut sink = [0u8; 64];
        while let Ok(n) = stream.read(&mut sink).await {
            if n == 0 {
                return;
            }
        }
        return;
    }
    let reason = match status {
        200 => "OK",
        429 => "Too Many Requests",
        418 => "I'm a teapot",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}
