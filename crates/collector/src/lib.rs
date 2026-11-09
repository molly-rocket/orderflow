pub mod binance;
pub mod checkpoint;
pub mod sink;
mod tracker;

use std::collections::HashMap;
use std::time::Duration;

use anyhow::bail;
use futures_util::{FutureExt, StreamExt};
use orderflow_domain::{GapReason, Market, MarketType, Trade, Venue};
pub use orderflow_runtime::Backoff;
use orderflow_runtime::Retry;
use tokio::net::TcpStream;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tokio_util::sync::CancellationToken;

use crate::checkpoint::{Checkpoint, Checkpoints};
use crate::sink::{Record, Sink};
use crate::tracker::Tracker;

const MAX_DRAIN: usize = 1_024;

#[derive(Debug, Clone)]
pub struct Config {
    pub venue_base_url: String,
    pub markets: Vec<Market>,
    pub stall_timeout: Duration,
    pub reconnect_min: Duration,
    pub reconnect_max: Duration,
    pub checkpoint_interval: Duration,
    pub retry: Backoff,
}

impl Config {
    fn validate(&self) -> anyhow::Result<()> {
        if self.markets.is_empty() {
            bail!("no markets configured");
        }
        if let Some(m) = self
            .markets
            .iter()
            .find(|m| m.venue() != Venue::Binance || m.market_type() != MarketType::Perp)
        {
            bail!(
                "market {m} is not a Binance perp market; this collector records Binance USD-M only"
            );
        }
        Ok(())
    }
}


pub async fn run<S: Sink, C: Checkpoints>(
    config: &Config,
    sink: &mut S,
    checkpoints: &C,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    config.validate()?;
    let mut out = Output {
        sink,
        checkpoints,
        retry: config.retry,
        shutdown: &shutdown,
    };
    let mut state = State::resume(config, &mut out).await?;
    let url = binance::stream_url(&config.venue_base_url, &config.markets);
    let mut backoff = config.reconnect_min;

    loop {
        let connected = tokio::select! {
            () = shutdown.cancelled() => break,
            result = tokio_tungstenite::connect_async(url.as_str()) => result,
        };
        match connected {
            Ok((ws, _)) => {
                tracing::info!(%url, "connected");
                match state.session(ws, config, &mut out).await? {
                    SessionEnd::Shutdown => break,
                    SessionEnd::Lapsed { received_trades } => {
                        if received_trades {
                            backoff = config.reconnect_min;
                        }
                    }
                }
            }
            Err(error) => tracing::warn!(%url, %error, "connect failed"),
        }
        state.lapse_all(GapReason::Disconnect, &mut out).await?;
        tokio::select! {
            () = shutdown.cancelled() => break,
            () = tokio::time::sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(config.reconnect_max);
    }

    state.save_checkpoints(&mut out).await
}

struct Output<'a, S, C> {
    sink: &'a mut S,
    checkpoints: &'a C,
    retry: Backoff,
    shutdown: &'a CancellationToken,
}

impl<S: Sink, C: Checkpoints> Output<'_, S, C> {
    async fn publish(&mut self, record: &Record) -> anyhow::Result<()> {
        let mut attempts = Retry::new(self.retry, self.shutdown, "publish");
        loop {
            match self.sink.publish(record).await {
                Ok(()) => {
                    attempts.succeeded();
                    return Ok(());
                }
                Err(error) if attempts.failed(&error).await => {}
                Err(_) => return Err(shut_down("publish")),
            }
        }
    }

    async fn load(&mut self, market: &Market) -> anyhow::Result<Option<Checkpoint>> {
        let what = format!("loading checkpoint for {market}");
        let mut attempts = Retry::new(self.retry, self.shutdown, &what);
        loop {
            match self.checkpoints.load(market).await {
                Ok(checkpoint) => {
                    attempts.succeeded();
                    return Ok(checkpoint);
                }
                Err(error) if attempts.failed(&error).await => {}
                Err(_) => return Err(shut_down(&what)),
            }
        }
    }

    async fn save(&mut self, market: &Market, checkpoint: &Checkpoint) -> anyhow::Result<()> {
        let what = format!("saving checkpoint for {market}");
        let mut attempts = Retry::new(self.retry, self.shutdown, &what);
        loop {
            match self.checkpoints.save(market, checkpoint).await {
                Ok(()) => {
                    attempts.succeeded();
                    return Ok(());
                }
                Err(error) if attempts.failed(&error).await => {}
                Err(_) => return Err(shut_down(&what)),
            }
        }
    }
}

fn shut_down(what: &str) -> anyhow::Error {
    anyhow::anyhow!("shut down before {what} succeeded")
}

enum SessionEnd {
    Shutdown,
    Lapsed { received_trades: bool },
}

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct State {
    trackers: Vec<Tracker>,
    by_symbol: HashMap<String, usize>,
    last_save: Instant,
}

impl State {
    async fn resume<S: Sink, C: Checkpoints>(
        config: &Config,
        out: &mut Output<'_, S, C>,
    ) -> anyhow::Result<Self> {
        let mut trackers = Vec::with_capacity(config.markets.len());
        let mut by_symbol = HashMap::with_capacity(config.markets.len());
        for market in &config.markets {
            let checkpoint = out.load(market).await?;
            let (tracker, opened) = Tracker::resume(market.clone(), checkpoint);
            if let Some(record) = opened {
                tracing::warn!(%market, "recorder downtime since last checkpoint");
                out.publish(&record).await?;
            }
            by_symbol.insert(market.native_name().to_owned(), trackers.len());
            trackers.push(tracker);
        }
        let mut state = Self {
            trackers,
            by_symbol,
            last_save: Instant::now(),
        };
        state.save_checkpoints(out).await?;
        Ok(state)
    }

    async fn session<S: Sink, C: Checkpoints>(
        &mut self,
        mut ws: Ws,
        config: &Config,
        out: &mut Output<'_, S, C>,
    ) -> anyhow::Result<SessionEnd> {
        let shutdown = out.shutdown;
        let mut pending = Vec::new();
        let mut received_trades = false;
        loop {
            let next = tokio::select! {
                () = shutdown.cancelled() => {
                    let _ = ws.close(None).await;
                    return Ok(SessionEnd::Shutdown);
                }
                next = tokio::time::timeout(config.stall_timeout, ws.next()) => next,
            };
            let mut open = match next {
                Ok(Some(Ok(message))) => self.handle(message, &mut pending),
                Ok(Some(Err(error))) => {
                    tracing::warn!(%error, "stream error");
                    false
                }
                Ok(None) => false,
                Err(_) => {
                    tracing::warn!(timeout = ?config.stall_timeout, "stream stalled");
                    false
                }
            };
            // Drain frames that are already buffered, without waiting.
            let mut drained = 0;
            while open && drained < MAX_DRAIN {
                match ws.next().now_or_never() {
                    Some(Some(Ok(message))) => open = self.handle(message, &mut pending),
                    Some(Some(Err(_)) | None) => open = false,
                    None => break,
                }
                drained += 1;
            }

            received_trades |= pending.iter().any(|r| matches!(r, Record::Trades { .. }));
            let gap_changed = pending.iter().any(|r| !matches!(r, Record::Trades { .. }));
            for record in pending.drain(..) {
                out.publish(&record).await?;
            }
            self.save_checkpoints_due(out, config, gap_changed).await?;
            if !open {
                return Ok(SessionEnd::Lapsed { received_trades });
            }
        }
    }

    fn handle(&mut self, message: Message, pending: &mut Vec<Record>) -> bool {
        match message {
            Message::Text(text) => {
                match binance::parse_frame(&text) {
                    Ok(binance::Frame::AggTrade { symbol, trade }) => {
                        self.accept(symbol, trade, pending);
                    }
                    Ok(binance::Frame::Other) => {}
                    Err(error) => tracing::warn!(%error, "unparseable frame skipped"),
                }
                true
            }
            Message::Close(_) => false,
            _ => true,
        }
    }

    fn accept(&mut self, symbol: &str, trade: Trade, pending: &mut Vec<Record>) {
        let Some(&index) = self.by_symbol.get(symbol) else {
            tracing::debug!(symbol, "trade for an unconfigured market");
            return;
        };
        let tracker = &mut self.trackers[index];
        if !tracker.observe(&trade, pending) {
            return; // duplicate
        }
        // Coalesce consecutive Trades of one Market into one batch.
        if let Some(Record::Trades { market, trades }) = pending.last_mut()
            && market == tracker.market()
        {
            trades.push(trade);
        } else {
            pending.push(Record::Trades {
                market: tracker.market().clone(),
                trades: vec![trade],
            });
        }
    }

    async fn lapse_all<S: Sink, C: Checkpoints>(
        &mut self,
        reason: GapReason,
        out: &mut Output<'_, S, C>,
    ) -> anyhow::Result<()> {
        let mut changed = false;
        for tracker in &mut self.trackers {
            if let Some(record) = tracker.lapse(reason) {
                tracing::warn!(market = %tracker.market(), ?reason, "gap opened");
                out.publish(&record).await?;
                changed = true;
            }
        }
        if changed {
            self.save_checkpoints(out).await?;
        }
        Ok(())
    }

    async fn save_checkpoints_due<S: Sink, C: Checkpoints>(
        &mut self,
        out: &mut Output<'_, S, C>,
        config: &Config,
        force: bool,
    ) -> anyhow::Result<()> {
        if force || self.last_save.elapsed() >= config.checkpoint_interval {
            self.save_checkpoints(out).await?;
        }
        Ok(())
    }

    async fn save_checkpoints<S: Sink, C: Checkpoints>(
        &mut self,
        out: &mut Output<'_, S, C>,
    ) -> anyhow::Result<()> {
        for tracker in &mut self.trackers {
            if let Some(checkpoint) = tracker.take_checkpoint() {
                out.save(tracker.market(), &checkpoint).await?;
            }
        }
        self.last_save = Instant::now();
        Ok(())
    }
}
