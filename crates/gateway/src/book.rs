use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::Response;
use futures_util::StreamExt;
use orderflow_bus::{Bus, decode};
use orderflow_contracts::book::{BookState, book_state_from_proto};
use orderflow_contracts::subjects::Namespace;
use orderflow_contracts::v1;
use orderflow_domain::{
    BookStatus, Ladder, Market, MarketSpec, PriceGrouping, PriceRow, Timestamp,
};
use serde::Deserialize;
use tokio::sync::{broadcast, watch};
use tokio::time::{Instant, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

use crate::dto::{BookDto, BookMessage, HeatmapFrameDto, LadderRowDto, decimal};
use crate::params::{ApiError, BarParams, MarketParam};
use crate::{AppState, now};

const SEND_INTERVAL: Duration = Duration::from_millis(200);

const STALE_AFTER: Duration = Duration::from_secs(3);

const RETENTION: Duration = Duration::from_secs(2 * 60 * 60);

const BASE_MULTIPLE: u32 = 10;

const BROADCAST_CAPACITY: usize = 64;

const MAX_HISTORY_FRAMES: i64 = 1_200;

#[derive(Debug, Clone)]
pub(crate) struct StoredFrame {
    time: Timestamp,
    status: BookStatus,
    low_row: Option<PriceRow>,
    sizes: Arc<[f32]>,
}

#[derive(Debug)]
pub(crate) struct HeatmapBuffer {
    base: PriceGrouping,
    frames: Mutex<VecDeque<StoredFrame>>,
    live: broadcast::Sender<StoredFrame>,
}

impl HeatmapBuffer {
    fn new(base: PriceGrouping) -> Self {
        Self {
            base,
            frames: Mutex::new(VecDeque::new()),
            live: broadcast::Sender::new(BROADCAST_CAPACITY),
        }
    }

    fn push(&self, state: &BookState) {
        let ladder = Ladder::new(&state.bids, &state.asks, state.known, self.base);
        let frame = match (state.status, ladder.known) {
            (BookStatus::Live, Some((first, last))) => {
                let len = usize::try_from(last.index() - first.index() + 1).unwrap_or(0);
                let mut sizes = vec![0f32; len];
                for row in &ladder.rows {
                    if let Ok(i) = usize::try_from(row.row.index() - first.index()) {
                        sizes[i] = (row.bid + row.ask).try_into().unwrap_or(f32::MAX);
                    }
                }
                StoredFrame {
                    time: state.time,
                    status: BookStatus::Live,
                    low_row: Some(first),
                    sizes: sizes.into(),
                }
            }
            _ => StoredFrame {
                time: state.time,
                status: state.status,
                low_row: None,
                sizes: Arc::from([]),
            },
        };
        let mut frames = self.frames.lock().unwrap_or_else(PoisonError::into_inner);
        let horizon = frame.time.as_micros() - micros(RETENTION);
        while frames.front().is_some_and(|f| f.time.as_micros() < horizon) {
            frames.pop_front();
        }
        frames.push_back(frame.clone());
        drop(frames);
        let _ = self.live.send(frame);
    }

    fn history(&self, since: Timestamp, column: Duration) -> Vec<StoredFrame> {
        let frames = self.frames.lock().unwrap_or_else(PoisonError::into_inner);
        let span = frames
            .back()
            .map_or(0, |newest| newest.time.as_micros() - since.as_micros());
        let column_us = micros(column)
            .max((span + MAX_HISTORY_FRAMES - 1) / MAX_HISTORY_FRAMES)
            .max(1);
        let mut out: Vec<StoredFrame> = Vec::new();
        for frame in frames.iter().filter(|f| f.time >= since) {
            let same_column = out.last().is_some_and(|last| {
                last.time.as_micros() / column_us == frame.time.as_micros() / column_us
            });
            if same_column {
                out.pop();
            }
            out.push(frame.clone());
        }
        out
    }

    fn dto(&self, frame: &StoredFrame, grouping: PriceGrouping) -> HeatmapFrameDto {
        let target = self.frame_grouping(grouping);
        let factor = usize::try_from(target.multiple() / self.base.multiple()).unwrap_or(1);
        let (low, sizes) = match frame.low_row {
            Some(low_row) if !frame.sizes.is_empty() => {
                let coarse_first = self.base.coarsen_row(low_row, &target);
                let offset = usize::try_from(
                    low_row.index() - coarse_first.index() * i64::try_from(factor).unwrap_or(1),
                )
                .unwrap_or(0);
                let len = (offset + frame.sizes.len()).div_ceil(factor);
                let mut sizes = vec![0f64; len];
                for (i, size) in frame.sizes.iter().enumerate() {
                    sizes[(offset + i) / factor] += f64::from(*size);
                }
                (
                    Some(decimal(target.row_low(coarse_first))),
                    sizes.into_iter().map(round_sig).collect(),
                )
            }
            _ => (None, Vec::new()),
        };
        HeatmapFrameDto {
            time_us: frame.time.as_micros(),
            status: frame.status.as_str(),
            step: decimal(target.step()),
            low,
            sizes,
        }
    }

    fn frame_grouping(&self, session: PriceGrouping) -> PriceGrouping {
        if session.multiple() > self.base.multiple() && self.base.can_coarsen_to(&session) {
            session
        } else {
            self.base
        }
    }
}

#[derive(Debug)]
pub(crate) struct MarketBooks {
    latest: watch::Sender<Option<(Instant, BookState)>>,
    heatmap: Arc<HeatmapBuffer>,
}

pub(crate) fn start(
    bus: &Bus,
    namespace: &Namespace,
    markets: &[MarketSpec],
    shutdown: &CancellationToken,
) -> HashMap<Market, MarketBooks> {
    let mut books = HashMap::new();
    for spec in markets {
        let base = PriceGrouping::new(spec.tick, BASE_MULTIPLE).unwrap_or_else(|_| {
            PriceGrouping::new(spec.tick, 1).expect("a tick size is a valid grouping")
        });
        let latest = watch::Sender::new(None);
        let heatmap = Arc::new(HeatmapBuffer::new(base));
        tokio::spawn(follow(
            bus.clone(),
            namespace.book(&spec.market),
            namespace.heatmap(&spec.market),
            latest.clone(),
            Arc::clone(&heatmap),
            shutdown.clone(),
        ));
        books.insert(spec.market.clone(), MarketBooks { latest, heatmap });
    }
    books
}

async fn follow(
    bus: Bus,
    book_subject: String,
    heatmap_subject: String,
    latest: watch::Sender<Option<(Instant, BookState)>>,
    heatmap: Arc<HeatmapBuffer>,
    shutdown: CancellationToken,
) {
    let subscribe = async {
        let books = bus.client().subscribe(book_subject.clone()).await?;
        let heatmaps = bus.client().subscribe(heatmap_subject.clone()).await?;
        anyhow::Ok((books, heatmaps))
    };
    let (mut books, mut heatmaps) = match subscribe.await {
        Ok(pair) => pair,
        Err(error) => {
            tracing::error!(%error, subject = %book_subject, "cannot follow the order book");
            return;
        }
    };
    loop {
        let (message, is_heatmap) = tokio::select! {
            () = shutdown.cancelled() => return,
            Some(m) = books.next() => (m, false),
            Some(m) = heatmaps.next() => (m, true),
            else => return,
        };
        let state = decode::<v1::BookState>(message.headers.as_ref(), &message.payload)
            .map_err(anyhow::Error::from)
            .and_then(|proto| Ok(book_state_from_proto(proto)?));
        match state {
            Ok(state) if is_heatmap => heatmap.push(&state),
            Ok(state) => {
                latest.send_replace(Some((Instant::now(), state)));
            }
            Err(error) => tracing::warn!(%error, "undecodable book state skipped"),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct BookParams {
    heatmap_since_us: Option<i64>,
}

pub(crate) async fn upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Query(market): Query<MarketParam>,
    Query(bar): Query<BarParams>,
    Query(book): Query<BookParams>,
) -> Result<Response, ApiError> {
    let spec = state.market(&market.market)?.clone();
    let bar = bar.validate(&spec)?;
    let books = state.books.get(&spec.market).ok_or_else(|| {
        ApiError::not_found(format!("market `{}` has no order book", spec.market))
    })?;
    let session = Session {
        grouping: bar.grouping,
        column: Duration::from_secs(u64::from(bar.timeframe.as_secs() / 60))
            .max(Duration::from_secs(1)),
        heatmap_since: book.heatmap_since_us.map(Timestamp::from_micros),
        latest: books.latest.subscribe(),
        heatmap: Arc::clone(&books.heatmap),
    };
    Ok(ws.on_upgrade(move |socket| async move {
        if let Err(error) = session.run(socket).await {
            tracing::debug!(%error, "book session ended");
        }
    }))
}

struct Session {
    grouping: PriceGrouping,
    column: Duration,
    heatmap_since: Option<Timestamp>,
    latest: watch::Receiver<Option<(Instant, BookState)>>,
    heatmap: Arc<HeatmapBuffer>,
}

impl Session {
    async fn run(mut self, mut socket: WebSocket) -> anyhow::Result<()> {
        let mut frames = self.heatmap_since.map(|_| self.heatmap.live.subscribe());
        if let Some(since) = self.heatmap_since {
            let history = self.heatmap.history(since, self.column);
            let frames = history
                .iter()
                .map(|f| self.heatmap.dto(f, self.grouping))
                .collect();
            send(&mut socket, &BookMessage::HeatmapHistory { frames }).await?;
        }
        let mut sent_status: Option<BookStatus> = None;
        let mut dirty = true;
        let mut ticker = tokio::time::interval(SEND_INTERVAL);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            let stale_check = tokio::time::sleep(STALE_AFTER);
            tokio::select! {
                incoming = socket.recv() => match incoming {
                    Some(Ok(Message::Close(_)) | Err(_)) | None => return Ok(()),
                    Some(Ok(_)) => {}
                },
                changed = self.latest.changed() => {
                    changed?;
                    dirty = true;
                    let status = self.latest.borrow().as_ref().map(|(_, s)| s.status);
                    if status != sent_status {
                        sent_status = status;
                        dirty = false;
                        self.send_book(&mut socket).await?;
                    }
                }
                _ = ticker.tick(), if dirty => {
                    dirty = false;
                    sent_status = self.latest.borrow().as_ref().map(|(_, s)| s.status);
                    self.send_book(&mut socket).await?;
                }
                () = stale_check, if !dirty && sent_status != Some(BookStatus::Disconnected) => {
                    if self.is_stale() {
                        sent_status = Some(BookStatus::Disconnected);
                        send(&mut socket, &BookMessage::Book { book: disconnected(self.grouping) }).await?;
                    }
                }
                frame = recv_frame(&mut frames) => {
                    let frame = self.heatmap.dto(&frame, self.grouping);
                    send(&mut socket, &BookMessage::Heatmap { frame }).await?;
                }
            }
        }
    }

    fn is_stale(&self) -> bool {
        self.latest
            .borrow()
            .as_ref()
            .is_none_or(|(received, _)| received.elapsed() >= STALE_AFTER)
    }

    async fn send_book(&self, socket: &mut WebSocket) -> anyhow::Result<()> {
        let book = match self.latest.borrow().as_ref() {
            Some((received, state)) if received.elapsed() < STALE_AFTER => {
                book_dto(state, self.grouping)
            }
            _ => disconnected(self.grouping),
        };
        send(socket, &BookMessage::Book { book }).await
    }
}

async fn recv_frame(frames: &mut Option<broadcast::Receiver<StoredFrame>>) -> StoredFrame {
    let Some(receiver) = frames else {
        return std::future::pending().await;
    };
    loop {
        match receiver.recv().await {
            Ok(frame) => return frame,
            Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return std::future::pending().await,
        }
    }
}

fn book_dto(state: &BookState, grouping: PriceGrouping) -> BookDto {
    if state.status != BookStatus::Live {
        return BookDto {
            time_us: state.time.as_micros(),
            ..disconnected_with(grouping, state.status)
        };
    }
    let ladder = Ladder::new(&state.bids, &state.asks, state.known, grouping);
    BookDto {
        time_us: state.time.as_micros(),
        status: BookStatus::Live.as_str(),
        best_bid: state
            .best_bid
            .as_ref()
            .or(state.bids.first())
            .map(|l| decimal(l.price)),
        best_ask: state
            .best_ask
            .as_ref()
            .or(state.asks.first())
            .map(|l| decimal(l.price)),
        known_low: state.known.map(|(low, _)| decimal(low)),
        known_high: state.known.map(|(_, high)| decimal(high)),
        step: decimal(grouping.step()),
        rows: ladder
            .rows
            .iter()
            .map(|r| LadderRowDto {
                price: decimal(r.low),
                bid: decimal(r.bid),
                ask: decimal(r.ask),
            })
            .collect(),
    }
}

fn disconnected(grouping: PriceGrouping) -> BookDto {
    disconnected_with(grouping, BookStatus::Disconnected)
}

fn disconnected_with(grouping: PriceGrouping, status: BookStatus) -> BookDto {
    BookDto {
        time_us: now().as_micros(),
        status: status.as_str(),
        best_bid: None,
        best_ask: None,
        known_low: None,
        known_high: None,
        step: decimal(grouping.step()),
        rows: Vec::new(),
    }
}

async fn send(socket: &mut WebSocket, message: &BookMessage) -> anyhow::Result<()> {
    let text = serde_json::to_string(message)?;
    socket.send(Message::Text(text.into())).await?;
    Ok(())
}

fn round_sig(value: f64) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return 0.0;
    }
    let digits = 3.0 - value.abs().log10().floor();
    let scale = 10f64.powf(digits);
    (value * scale).round() / scale
}

fn micros(duration: Duration) -> i64 {
    i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use orderflow_domain::{Level, TickSize};

    use super::*;

    fn level(price: &str, size: &str) -> Level {
        Level {
            price: price.parse().unwrap(),
            size: size.parse().unwrap(),
        }
    }

    fn live_state(time_us: i64) -> BookState {
        BookState {
            market: "binance:perp:BTCUSDT".parse().unwrap(),
            time: Timestamp::from_micros(time_us),
            status: BookStatus::Live,
            bids: vec![level("100.0", "1")],
            asks: vec![level("100.1", "2")],
            known: Some(("99.0".parse().unwrap(), "101.0".parse().unwrap())),
            best_bid: Some(level("101.5", "3")),
            best_ask: Some(level("101.6", "4")),
        }
    }

    fn grouping(multiple: u32) -> PriceGrouping {
        PriceGrouping::new(TickSize::new("0.1".parse().unwrap()).unwrap(), multiple).unwrap()
    }

    #[test]
    fn the_dto_reports_the_unfiltered_best_bid_and_ask() {
        let dto = book_dto(&live_state(0), grouping(1));

        assert_eq!(dto.best_bid.as_deref(), Some("101.5"));
        assert_eq!(dto.best_ask.as_deref(), Some("101.6"));
        assert_eq!(dto.known_high.as_deref(), Some("101"));
    }

    #[test]
    fn history_is_capped_by_widening_its_columns() {
        let buffer = HeatmapBuffer::new(grouping(10));
        for second in 0..3_000 {
            buffer.push(&live_state(second * 1_000_000));
        }

        let history = buffer.history(Timestamp::from_micros(0), Duration::from_secs(1));

        assert!(history.len() <= 1_200, "{} frames", history.len());
        assert!(history.len() >= 1_000, "{} frames", history.len());
        assert_eq!(
            history.last().map(|f| f.time.as_micros()),
            Some(2_999 * 1_000_000)
        );
    }
}
