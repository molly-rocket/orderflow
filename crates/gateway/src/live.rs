use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_nats::jetstream::consumer::{DeliverPolicy, pull};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::Response;
use futures_util::StreamExt;
use orderflow_bus::decode_record;
use orderflow_contracts::record::Record;
use orderflow_domain::{FootprintBar, Gap, TimeRange, Timestamp, Trade};
use serde::Deserialize;
use tokio::time::MissedTickBehavior;

use crate::params::{ApiError, BarParams, BarSpec, MarketParam};
use crate::{AppState, BarDto, GapDto, LiveMessage, MarketSpec};

const UPDATE_INTERVAL: Duration = Duration::from_millis(100);

const DELIVERY_LEAD: Duration = Duration::from_secs(5);

const MAX_LOOKBACK: Duration = Duration::from_secs(23 * 60 * 60);

#[derive(Debug, Deserialize)]
pub(crate) struct LiveParams {
    since_us: Option<i64>,
}

pub(crate) async fn upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Query(market): Query<MarketParam>,
    Query(bar): Query<BarParams>,
    Query(live): Query<LiveParams>,
) -> Result<Response, ApiError> {
    let spec = state.market(&market.market)?.clone();
    let bar = bar.validate(&spec)?;
    let now = now();
    let since = live.since_us.map_or(now, Timestamp::from_micros);
    let oldest = Timestamp::from_micros(now.as_micros() - micros(MAX_LOOKBACK));
    if since < oldest || since > now {
        return Err(ApiError::bad_request(
            "since_us must be within the last 23 hours and not in the future",
        ));
    }
    Ok(ws.on_upgrade(move |socket| async move {
        if let Err(error) = Session::new(&state, spec, bar, since).run(socket).await {
            tracing::debug!(%error, "live session ended");
        }
    }))
}

struct Session<'a> {
    state: &'a AppState,
    spec: MarketSpec,
    bar_spec: BarSpec,
    first_bar_start: Timestamp,
    current: Option<FootprintBar>,
    gaps: Vec<(String, Gap)>,
    dirty: bool,
}

impl<'a> Session<'a> {
    fn new(state: &'a AppState, spec: MarketSpec, bar_spec: BarSpec, since: Timestamp) -> Self {
        Self {
            state,
            first_bar_start: bar_spec.timeframe.bar_start(since),
            spec,
            bar_spec,
            current: None,
            gaps: Vec::new(),
            dirty: false,
        }
    }

    async fn run(mut self, mut socket: WebSocket) -> anyhow::Result<()> {
        let namespace = &self.state.namespace;
        let market = &self.spec.market;
        let stream = self
            .state
            .bus
            .jetstream()
            .get_stream(namespace.record_stream())
            .await?;
        let deliver_from = self.first_bar_start.as_micros() - micros(DELIVERY_LEAD);
        let consumer = stream
            .create_consumer(pull::OrderedConfig {
                filter_subjects: vec![namespace.trades(market), namespace.gaps(market)],
                deliver_policy: DeliverPolicy::ByStartTime {
                    start_time: time::OffsetDateTime::from_unix_timestamp_nanos(
                        i128::from(deliver_from) * 1_000,
                    )?,
                },
                ..Default::default()
            })
            .await?;
        let mut messages = consumer.messages().await?;

        let window = TimeRange::new(self.first_bar_start, Timestamp::from_micros(i64::MAX))
            .expect("start is before the end of time");
        for stored in self.state.store.gaps_with_ids(market, window).await? {
            send(
                &mut socket,
                &LiveMessage::Gap {
                    gap: (&stored.gap).into(),
                },
            )
            .await?;
            self.gaps.push((stored.id, stored.gap));
        }

        let mut ticker = tokio::time::interval(UPDATE_INTERVAL);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                incoming = socket.recv() => match incoming {
                    Some(Ok(Message::Close(_)) | Err(_)) | None => return Ok(()),
                    Some(Ok(_)) => {}
                },
                next = messages.next() => {
                    let Some(message) = next else { return Ok(()) };
                    let message = message?;
                    match decode_record(message.headers.as_ref(), &message.payload) {
                        Ok(record) => self.apply(record, &mut socket).await?,
                        Err(error) => tracing::warn!(%error, "undecodable record skipped"),
                    }
                }
                _ = ticker.tick(), if self.dirty => {
                    if let Some(bar) = &self.current {
                        let message = LiveMessage::Bar { bar: self.dto(bar), closed: false };
                        send(&mut socket, &message).await?;
                    }
                    self.dirty = false;
                }
            }
        }
    }

    async fn apply(&mut self, record: Record, socket: &mut WebSocket) -> anyhow::Result<()> {
        match record {
            Record::Trades { trades, .. } => {
                for trade in &trades {
                    self.add_trade(trade, socket).await?;
                }
            }
            Record::GapOpened {
                gap_id,
                start,
                reason,
                ..
            } => {
                if !self.gaps.iter().any(|(id, _)| *id == gap_id) {
                    let gap = Gap::open(start, reason);
                    send(socket, &LiveMessage::Gap { gap: (&gap).into() }).await?;
                    self.gaps.push((gap_id, gap));
                    self.dirty = true;
                }
            }
            Record::GapClosed { gap_id, end, .. } => {
                if let Some((_, gap)) = self.gaps.iter_mut().find(|(id, _)| *id == gap_id)
                    && let Some(closed) = gap.close(end)
                {
                    *gap = closed;
                    send(
                        socket,
                        &LiveMessage::Gap {
                            gap: GapDto::from(&closed),
                        },
                    )
                    .await?;
                    self.dirty = true;
                }
            }
        }
        Ok(())
    }

    async fn add_trade(&mut self, trade: &Trade, socket: &mut WebSocket) -> anyhow::Result<()> {
        let time = trade.engine_time;
        if time < self.first_bar_start {
            return Ok(());
        }
        if let Some(bar) = &self.current
            && time >= bar.range().end()
        {
            let message = LiveMessage::Bar {
                bar: self.dto(bar),
                closed: true,
            };
            send(socket, &message).await?;
            self.current = None;
        }
        let bar = self.current.get_or_insert_with(|| {
            FootprintBar::new(
                self.bar_spec.timeframe.bar_range(time),
                self.bar_spec.grouping,
            )
        });
        if bar.add(trade).is_ok() {
            self.dirty = true;
        }
        Ok(())
    }

    fn dto(&self, bar: &FootprintBar) -> BarDto {
        let gaps: Vec<Gap> = self.gaps.iter().map(|(_, g)| *g).collect();
        BarDto::new(bar, &self.bar_spec.imbalance, &gaps)
    }
}

async fn send(socket: &mut WebSocket, message: &LiveMessage) -> anyhow::Result<()> {
    let text = serde_json::to_string(message)?;
    socket.send(Message::Text(text.into())).await?;
    Ok(())
}

fn now() -> Timestamp {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp::from_micros(i64::try_from(since_epoch.as_micros()).unwrap_or(i64::MAX))
}

fn micros(duration: Duration) -> i64 {
    i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
}
