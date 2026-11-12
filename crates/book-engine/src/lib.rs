pub mod binance;
pub mod sync;

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use futures_util::StreamExt;
use orderflow_bus::Bus;
use orderflow_contracts::book::{BookState, book_state_to_proto};
use orderflow_contracts::subjects::Namespace;
use orderflow_domain::{BookSide, BookStatus, MarketSpec, Timestamp};
use orderflow_runtime::{Backoff, Retry};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tokio_util::sync::CancellationToken;

use crate::binance::{Snapshot, parse_frame, parse_snapshot};
use crate::sync::{Action, BookSync};

const PUBLISH_INTERVAL: Duration = Duration::from_millis(100);

const HEARTBEAT: Duration = Duration::from_secs(1);

const HEATMAP_INTERVAL: Duration = Duration::from_secs(1);

const MAX_SNAPSHOT_ATTEMPTS: u32 = 5;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Debug, Clone)]
pub struct Config {
    pub namespace: Namespace,
    pub markets: Vec<MarketSpec>,
    pub ws_url: String,
    pub rest_url: String,
    pub stall_timeout: Duration,
    pub snapshot_timeout: Duration,
    pub retry: Backoff,
    pub max_buffer: usize,
}

struct MarketBook {
    spec: MarketSpec,
    sync: BookSync,
    published_status: Option<BookStatus>,
    dirty: bool,
    last_published: Option<Instant>,
    snapshot_in_flight: bool,
}

struct Engine<'a> {
    config: &'a Config,
    bus: &'a Bus,
    shutdown: &'a CancellationToken,
    books: Vec<MarketBook>,
    by_symbol: HashMap<String, usize>,
    snapshots: mpsc::Sender<(usize, Option<Snapshot>)>,
}

enum SessionEnd {
    Shutdown,
    Lapsed,
}

pub async fn run(config: &Config, bus: &Bus, shutdown: CancellationToken) -> anyhow::Result<()> {
    if config.markets.is_empty() {
        bail!("no markets configured");
    }
    let (snapshots, mut snapshot_rx) = mpsc::channel(16);
    let mut engine = Engine {
        config,
        bus,
        shutdown: &shutdown,
        books: config
            .markets
            .iter()
            .map(|spec| MarketBook {
                spec: spec.clone(),
                sync: BookSync::new(config.max_buffer),
                published_status: None,
                dirty: true,
                last_published: None,
                snapshot_in_flight: false,
            })
            .collect(),
        by_symbol: config
            .markets
            .iter()
            .enumerate()
            .map(|(i, spec)| (spec.market.native_name().to_owned(), i))
            .collect(),
        snapshots,
    };
    let markets: Vec<_> = config.markets.iter().map(|s| s.market.clone()).collect();
    let url = binance::stream_url(&config.ws_url, &markets);
    let mut backoff = config.retry.min;
    loop {
        let connected = tokio::select! {
            () = shutdown.cancelled() => break,
            result = tokio_tungstenite::connect_async(url.as_str()) => result,
        };
        match connected {
            Ok((ws, _)) => {
                tracing::info!(%url, "connected");
                backoff = config.retry.min;
                match engine.session(ws, &mut snapshot_rx).await? {
                    SessionEnd::Shutdown => break,
                    SessionEnd::Lapsed => {}
                }
            }
            Err(error) => tracing::warn!(%url, %error, "connect failed"),
        }
        engine.disconnected().await?;
        tokio::select! {
            () = shutdown.cancelled() => break,
            () = tokio::time::sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(config.retry.max);
    }
    Ok(())
}

impl Engine<'_> {
    async fn session(
        &mut self,
        mut ws: Ws,
        snapshot_rx: &mut mpsc::Receiver<(usize, Option<Snapshot>)>,
    ) -> anyhow::Result<SessionEnd> {
        let mut last_message = Instant::now();
        let mut publish = tokio::time::interval(PUBLISH_INTERVAL);
        publish.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut heatmap = tokio::time::interval(HEATMAP_INTERVAL);
        heatmap.set_missed_tick_behavior(MissedTickBehavior::Delay);
        heatmap.tick().await;
        loop {
            tokio::select! {
                () = self.shutdown.cancelled() => {
                    let _ = ws.close(None).await;
                    return Ok(SessionEnd::Shutdown);
                }
                next = ws.next() => match next {
                    Some(Ok(message)) => {
                        last_message = Instant::now();
                        if !self.handle(message) {
                            return Ok(SessionEnd::Lapsed);
                        }
                        self.publish_status_changes().await?;
                    }
                    Some(Err(error)) => {
                        tracing::warn!(%error, "stream error");
                        return Ok(SessionEnd::Lapsed);
                    }
                    None => return Ok(SessionEnd::Lapsed),
                },
                Some((index, result)) = snapshot_rx.recv() => {
                    let book = &mut self.books[index];
                    book.snapshot_in_flight = false;
                    book.dirty = true;
                    let action = match result {
                        Some(snapshot) => book.sync.on_snapshot(snapshot, now()),
                        None => book.sync.on_snapshot_failed(),
                    };
                    self.act(index, action);
                    self.publish_status_changes().await?;
                }
                _ = publish.tick() => {
                    if last_message.elapsed() >= self.config.stall_timeout {
                        tracing::warn!(timeout = ?self.config.stall_timeout, "stream stalled");
                        let _ = ws.close(None).await;
                        return Ok(SessionEnd::Lapsed);
                    }
                    self.publish_due().await?;
                }
                _ = heatmap.tick() => self.publish_heatmaps().await?,
            }
        }
    }

    fn handle(&mut self, message: Message) -> bool {
        let text = match message {
            Message::Text(text) => text,
            Message::Close(_) => return false,
            _ => return true,
        };
        let diff = match parse_frame(&text) {
            Ok(Some(diff)) => diff,
            Ok(None) => return true,
            Err(error) => {
                tracing::warn!(%error, "undecodable depth frame skipped");
                return true;
            }
        };
        let Some(&index) = self.by_symbol.get(&diff.symbol) else {
            return true;
        };
        let book = &mut self.books[index];
        let action = book.sync.on_diff(diff, now());
        book.dirty = true;
        self.act(index, action);
        true
    }

    fn act(&mut self, index: usize, action: Option<Action>) {
        let Some(Action::FetchSnapshot) = action else {
            return;
        };
        let book = &mut self.books[index];
        if book.snapshot_in_flight {
            return;
        }
        book.snapshot_in_flight = true;
        let url = binance::snapshot_url(&self.config.rest_url, &book.spec.market);
        let backoff = self.config.retry;
        let timeout = self.config.snapshot_timeout;
        let shutdown = self.shutdown.clone();
        let sender = self.snapshots.clone();
        tokio::spawn(async move {
            let mut attempts = Retry::new(backoff, &shutdown, "fetch depth snapshot");
            for attempt in 1..=MAX_SNAPSHOT_ATTEMPTS {
                let fetched = tokio::task::spawn_blocking({
                    let url = url.clone();
                    move || fetch_snapshot(&url, timeout)
                })
                .await
                .map_err(anyhow::Error::from)
                .and_then(|r| r);
                match fetched {
                    Ok(snapshot) => {
                        attempts.succeeded();
                        let _ = sender.send((index, Some(snapshot))).await;
                        return;
                    }
                    Err(_) if attempt == MAX_SNAPSHOT_ATTEMPTS => {}
                    Err(error) if attempts.failed(&error).await => {}
                    Err(_) => return,
                }
            }
            tracing::warn!(attempts = MAX_SNAPSHOT_ATTEMPTS, "depth snapshot abandoned");
            let _ = sender.send((index, None)).await;
        });
    }

    async fn publish_status_changes(&mut self) -> anyhow::Result<()> {
        for index in 0..self.books.len() {
            let book = &self.books[index];
            if book.published_status != Some(book.sync.status()) {
                self.publish_book(index, Instant::now()).await?;
                let subject = self
                    .config
                    .namespace
                    .heatmap(&self.books[index].spec.market);
                self.publish_state(index, &subject).await?;
            }
        }
        Ok(())
    }

    async fn publish_book(&mut self, index: usize, now: Instant) -> anyhow::Result<()> {
        let subject = self.config.namespace.book(&self.books[index].spec.market);
        self.publish_state(index, &subject).await?;
        let book = &mut self.books[index];
        book.dirty = false;
        book.last_published = Some(now);
        book.published_status = Some(book.sync.status());
        Ok(())
    }

    async fn publish_due(&mut self) -> anyhow::Result<()> {
        let now = Instant::now();
        for index in 0..self.books.len() {
            let book = &self.books[index];
            let stale = book
                .last_published
                .is_none_or(|last| now.duration_since(last) >= HEARTBEAT);
            if book.dirty || stale {
                self.publish_book(index, now).await?;
            }
        }
        Ok(())
    }

    async fn publish_heatmaps(&mut self) -> anyhow::Result<()> {
        for index in 0..self.books.len() {
            let subject = self
                .config
                .namespace
                .heatmap(&self.books[index].spec.market);
            self.publish_state(index, &subject).await?;
        }
        Ok(())
    }

    async fn publish_state(&self, index: usize, subject: &str) -> anyhow::Result<()> {
        let book = &self.books[index];
        let state = match book.sync.book() {
            Some(order_book) => BookState {
                market: book.spec.market.clone(),
                time: now(),
                status: BookStatus::Live,
                bids: order_book.levels(BookSide::Bid),
                asks: order_book.levels(BookSide::Ask),
                known: order_book.known_range(),
                best_bid: order_book.best_bid(),
                best_ask: order_book.best_ask(),
            },
            None => BookState {
                market: book.spec.market.clone(),
                time: now(),
                status: book.sync.status(),
                bids: Vec::new(),
                asks: Vec::new(),
                known: None,
                best_bid: None,
                best_ask: None,
            },
        };
        let proto = book_state_to_proto(&state)?;
        self.bus
            .publish_core(subject, &proto)
            .await
            .with_context(|| format!("publishing {subject}"))
    }

    async fn disconnected(&mut self) -> anyhow::Result<()> {
        for book in &mut self.books {
            book.sync.on_disconnect();
            book.dirty = true;
        }
        self.publish_status_changes().await?;
        self.publish_due().await
    }
}

fn fetch_snapshot(url: &str, timeout: Duration) -> anyhow::Result<Snapshot> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into();
    let body = agent
        .get(url)
        .call()
        .with_context(|| format!("GET {url}"))?
        .body_mut()
        .with_config()
        .limit(16 * 1024 * 1024)
        .read_to_string()
        .with_context(|| format!("reading {url}"))?;
    parse_snapshot(&body).with_context(|| format!("decoding {url}"))
}

fn now() -> Timestamp {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp::from_micros(i64::try_from(since_epoch.as_micros()).unwrap_or(i64::MAX))
}
