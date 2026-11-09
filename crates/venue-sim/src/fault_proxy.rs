use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub struct FaultProxy {
    addr: SocketAddr,
    state: Arc<State>,
    accept: JoinHandle<()>,
}

#[derive(Debug)]
struct State {
    up: AtomicBool,
    connections: Mutex<CancellationToken>,
}

impl FaultProxy {
    pub async fn start(upstream: String) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let state = Arc::new(State {
            up: AtomicBool::new(true),
            connections: Mutex::new(CancellationToken::new()),
        });
        let accept = tokio::spawn(accept_loop(listener, upstream, Arc::clone(&state)));
        Ok(Self {
            addr,
            state,
            accept,
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn cut(&self) {
        self.state.up.store(false, Ordering::SeqCst);
        let mut connections = self
            .state
            .connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        connections.cancel();
        *connections = CancellationToken::new();
    }

    pub fn restore(&self) {
        self.state.up.store(true, Ordering::SeqCst);
    }
}

impl Drop for FaultProxy {
    fn drop(&mut self) {
        self.accept.abort();
        self.cut();
    }
}

async fn accept_loop(listener: TcpListener, upstream: String, state: Arc<State>) {
    while let Ok((mut client, _)) = listener.accept().await {
        if !state.up.load(Ordering::SeqCst) {
            continue;
        }
        let cut = state
            .connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let upstream = upstream.clone();
        tokio::spawn(async move {
            let Ok(mut server) = TcpStream::connect(&upstream).await else {
                return;
            };
            tokio::select! {
                () = cut.cancelled() => {}
                _ = tokio::io::copy_bidirectional(&mut client, &mut server) => {}
            }
        });
    }
}
