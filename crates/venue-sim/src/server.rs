use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, PoisonError};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

use crate::Fixture;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pacing {
    AsFastAsPossible,
    Realtime { speed: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    Disconnect {
        after_frames: usize,
        lost_frames: usize,
    },
    Drop { at_frame: usize, count: usize },
}

#[derive(Debug, Clone)]
pub struct Script {
    fixture: Fixture,
    pacing: Pacing,
    faults: Vec<Fault>,
}

impl Script {
    pub fn new(fixture: Fixture) -> Self {
        Self {
            fixture,
            pacing: Pacing::AsFastAsPossible,
            faults: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_pacing(mut self, pacing: Pacing) -> Self {
        self.pacing = pacing;
        self
    }

    #[must_use]
    pub fn with_fault(mut self, fault: Fault) -> Self {
        self.faults.push(fault);
        self
    }

    fn is_dropped(&self, index: usize) -> bool {
        self.faults.iter().any(|fault| {
            matches!(*fault, Fault::Drop { at_frame, count } if (at_frame..at_frame + count).contains(&index))
        })
    }

    fn disconnect_at(&self, index: usize) -> Option<usize> {
        self.faults.iter().find_map(|fault| match *fault {
            Fault::Disconnect {
                after_frames,
                lost_frames,
            } if after_frames == index => Some(lost_frames),
            _ => None,
        })
    }
}

#[derive(Debug, Default)]
struct Cursor {
    next: usize,
    fired: Vec<usize>,
}

#[derive(Debug)]
struct Shared {
    script: Script,
    cursor: Mutex<Cursor>,
    connections: AtomicUsize,
    paths: StdMutex<Vec<String>>,
    finished: watch::Sender<bool>,
}

#[derive(Debug)]
pub struct FakeVenue {
    addr: SocketAddr,
    shared: Arc<Shared>,
    accept_loop: JoinHandle<()>,
}

impl FakeVenue {
    pub async fn start(script: Script) -> std::io::Result<Self> {
        Self::bind(script, SocketAddr::from(([127, 0, 0, 1], 0))).await
    }

    pub async fn bind(script: Script, addr: SocketAddr) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared {
            finished: watch::Sender::new(script.fixture.is_empty()),
            script,
            cursor: Mutex::default(),
            connections: AtomicUsize::new(0),
            paths: StdMutex::default(),
        });
        let accept_loop = tokio::spawn(accept_loop(listener, Arc::clone(&shared)));
        Ok(Self {
            addr,
            shared,
            accept_loop,
        })
    }

    pub fn url(&self) -> String {
        format!("ws://{}", self.addr)
    }

    pub fn connections(&self) -> usize {
        self.shared.connections.load(Ordering::SeqCst)
    }

    pub fn requested_paths(&self) -> Vec<String> {
        self.shared
            .paths
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub async fn finished(&self) {
        let mut rx = self.shared.finished.subscribe();
        let _ = rx.wait_for(|done| *done).await;
    }
}

impl Drop for FakeVenue {
    fn drop(&mut self) {
        self.accept_loop.abort();
    }
}

async fn accept_loop(listener: TcpListener, shared: Arc<Shared>) {
    let mut sessions = tokio::task::JoinSet::new();
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        sessions.spawn(serve_session(stream, Arc::clone(&shared)));
        while sessions.try_join_next().is_some() {}
    }
}

#[expect(
    clippy::result_large_err,
    reason = "tungstenite fixes the handshake callback signature"
)]
async fn serve_session(stream: TcpStream, shared: Arc<Shared>) {
    let mut path = String::new();
    let record_path = |request: &Request, response: Response| {
        path = request
            .uri()
            .path_and_query()
            .map_or_else(|| "/".to_owned(), ToString::to_string);
        Ok(response)
    };
    let Ok(mut ws) = tokio_tungstenite::accept_hdr_async(stream, record_path).await else {
        return;
    };

    let mut cursor = shared.cursor.lock().await;
    shared.connections.fetch_add(1, Ordering::SeqCst);
    shared
        .paths
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(path);

    let script = &shared.script;
    let frames = script.fixture.frames();
    let mut previous_recv_us: Option<i64> = None;

    while cursor.next < frames.len() {
        let index = cursor.next;
        if !cursor.fired.contains(&index)
            && let Some(lost) = script.disconnect_at(index)
        {
            cursor.fired.push(index);
            cursor.next = (index + lost).min(frames.len());
            mark_finished_if_done(&shared, &cursor);
            let _ = ws.close(None).await;
            return;
        }
        cursor.next += 1;
        if script.is_dropped(index) {
            continue;
        }
        let frame = &frames[index];
        if let (Pacing::Realtime { speed }, Some(previous)) = (script.pacing, previous_recv_us) {
            tokio::time::sleep(spacing(previous, frame.recv_us, speed)).await;
        }
        previous_recv_us = Some(frame.recv_us);
        if ws.send(Message::text(frame.text.as_str())).await.is_err() {
            return; // client went away; the cursor keeps the position, like a live Venue.
        }
    }
    mark_finished_if_done(&shared, &cursor);
    drop(cursor);

    // Idle like a quiet live Venue: keep reading so pings are answered and closes are seen.
    while let Some(Ok(message)) = ws.next().await {
        if message.is_close() {
            break;
        }
    }
}

fn mark_finished_if_done(shared: &Shared, cursor: &Cursor) {
    if cursor.next >= shared.script.fixture.len() {
        shared.finished.send_replace(true);
    }
}

fn spacing(previous_us: i64, next_us: i64, speed: f64) -> Duration {
    let gap_us = u64::try_from(next_us.saturating_sub(previous_us)).unwrap_or(0);
    if speed <= 0.0 {
        return Duration::ZERO;
    }
    Duration::from_micros(gap_us).div_f64(speed)
}
