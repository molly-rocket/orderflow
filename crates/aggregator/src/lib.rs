use std::collections::HashMap;
use std::time::Duration;

use anyhow::bail;
use async_nats::jetstream::consumer::{DeliverPolicy, pull};
use async_nats::jetstream::stream::{LastRawMessageErrorKind, Stream};
use futures_util::StreamExt;
use orderflow_bus::{Bus, StreamSpec, decode_record};
use orderflow_contracts::record::Record;
use orderflow_contracts::subjects::Namespace;
use orderflow_domain::{Bubble, BubbleBuilder, BubbleGrouping, Market, Trade};
use orderflow_runtime::{Backoff, Retry, retry};
use tokio::time::{Instant, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

const IDLE_CHECK: Duration = Duration::from_millis(250);

const IDLE_CLOSE: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub struct Config {
    pub namespace: Namespace,
    pub markets: Vec<Market>,
    pub grouping: BubbleGrouping,
    pub retry: Backoff,
}

pub async fn run(config: &Config, bus: &Bus, shutdown: CancellationToken) -> anyhow::Result<()> {
    if config.markets.is_empty() {
        bail!("no markets configured");
    }
    let stream = bus
        .ensure_stream(&StreamSpec::record(&config.namespace))
        .await?;
    loop {
        let Some(mut markets) = retry(config.retry, &shutdown, "read published bubbles", || {
            Resume::load(config, &stream)
        })
        .await
        else {
            return Ok(());
        };
        let from = markets.start_sequence();
        let Some(messages) = retry(config.retry, &shutdown, "open the record stream", || {
            open(&stream, config, from)
        })
        .await
        else {
            return Ok(());
        };
        tracing::info!(stream = %config.namespace.record_stream(), from, "aggregating");

        match markets.consume(config, bus, &shutdown, messages).await {
            Consumed::Shutdown => return Ok(()),
            Consumed::Interrupted(error) => tracing::warn!(
                error = %format!("{error:#}"),
                "record stream interrupted; resuming from the published bubbles"
            ),
        }
    }
}

async fn open(
    stream: &Stream,
    config: &Config,
    start_sequence: u64,
) -> anyhow::Result<pull::Ordered> {
    let filter_subjects = config
        .markets
        .iter()
        .flat_map(|m| [config.namespace.trades(m), config.namespace.gaps(m)])
        .collect();
    let consumer = stream
        .create_consumer(pull::OrderedConfig {
            filter_subjects,
            deliver_policy: DeliverPolicy::ByStartSequence { start_sequence },
            ..Default::default()
        })
        .await?;
    Ok(consumer.messages().await?)
}

enum Consumed {
    Shutdown,
    Interrupted(anyhow::Error),
}

struct MarketState {
    builder: BubbleBuilder,
    published_through: Option<u64>,
    resume_from: u64,
    open_since: Option<u64>,
    last_seq: u64,
    last_activity: Instant,
}

struct Resume {
    markets: HashMap<Market, MarketState>,
}

impl Resume {
    async fn load(config: &Config, stream: &Stream) -> anyhow::Result<Self> {
        let first_available = stream.get_info().await?.state.first_sequence;
        let mut markets = HashMap::with_capacity(config.markets.len());
        for market in &config.markets {
            let (published_through, resume_from) = match stream
                .get_last_raw_message_by_subject(&config.namespace.bubbles(market))
                .await
            {
                Ok(message) => match decode_record(Some(&message.headers), &message.payload)? {
                    Record::Bubbles {
                        bubbles,
                        resume_from_seq,
                        ..
                    } => (bubbles.last().map(|b| b.last_trade_id), resume_from_seq),
                    other => bail!("unexpected record on the bubble subject: {other:?}"),
                },
                Err(error) if matches!(error.kind(), LastRawMessageErrorKind::NoMessageFound) => {
                    (None, first_available)
                }
                Err(error) => return Err(error.into()),
            };
            markets.insert(
                market.clone(),
                MarketState {
                    builder: BubbleBuilder::new(config.grouping),
                    published_through,
                    resume_from: resume_from.max(first_available).max(1),
                    open_since: None,
                    last_seq: 0,
                    last_activity: Instant::now(),
                },
            );
        }
        Ok(Self { markets })
    }

    fn start_sequence(&self) -> u64 {
        self.markets
            .values()
            .map(|m| m.resume_from)
            .min()
            .unwrap_or(1)
    }

    async fn consume(
        &mut self,
        config: &Config,
        bus: &Bus,
        shutdown: &CancellationToken,
        mut messages: pull::Ordered,
    ) -> Consumed {
        let mut idle = tokio::time::interval(IDLE_CHECK);
        idle.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            let next = tokio::select! {
                () = shutdown.cancelled() => return Consumed::Shutdown,
                _ = idle.tick() => {
                    for batch in self.close_idle() {
                        if !publish(config, bus, shutdown, &batch).await {
                            return Consumed::Shutdown;
                        }
                    }
                    continue;
                }
                next = messages.next() => next,
            };
            let message = match next {
                Some(Ok(message)) => message,
                Some(Err(error)) => return Consumed::Interrupted(error.into()),
                None => return Consumed::Interrupted(anyhow::anyhow!("record stream ended")),
            };
            let seq = match message.info() {
                Ok(info) => info.stream_sequence,
                Err(error) => return Consumed::Interrupted(anyhow::anyhow!(error)),
            };
            let record = match decode_record(message.headers.as_ref(), &message.payload) {
                Ok(record) => record,
                Err(error) => {
                    tracing::error!(seq, %error, "skipping undecodable record");
                    continue;
                }
            };
            if let Some(batch) = self.apply(record, seq)
                && !publish(config, bus, shutdown, &batch).await
            {
                return Consumed::Shutdown;
            }
        }
    }

    fn close_idle(&mut self) -> Vec<Record> {
        self.markets
            .iter_mut()
            .filter(|(_, state)| state.last_activity.elapsed() >= IDLE_CLOSE)
            .filter_map(|(market, state)| {
                let closed = state.builder.close()?;
                state.open_since = None;
                Some(Record::Bubbles {
                    market: market.clone(),
                    bubbles: vec![closed],
                    resume_from_seq: state.last_seq + 1,
                })
            })
            .collect()
    }

    fn apply(&mut self, record: Record, seq: u64) -> Option<Record> {
        match record {
            Record::Trades { market, trades } => {
                let state = self.markets.get_mut(&market)?;
                let closed = state.absorb(&trades, seq);
                (!closed.is_empty()).then(|| Record::Bubbles {
                    market,
                    bubbles: closed,
                    resume_from_seq: state.open_since.unwrap_or(seq + 1),
                })
            }
            Record::GapOpened { market, .. } => {
                let state = self.markets.get_mut(&market)?;
                let closed = state.builder.close()?;
                state.open_since = None;
                Some(Record::Bubbles {
                    market,
                    bubbles: vec![closed],
                    resume_from_seq: seq + 1,
                })
            }
            Record::GapClosed { .. } | Record::Bubbles { .. } => None,
        }
    }
}

impl MarketState {
    fn absorb(&mut self, trades: &[Trade], seq: u64) -> Vec<Bubble> {
        self.last_seq = seq;
        self.last_activity = Instant::now();
        let mut closed = Vec::new();
        for trade in trades {
            if self
                .published_through
                .is_some_and(|through| trade.venue_trade_id <= through)
            {
                continue;
            }
            closed.extend(self.builder.push(trade));
            if self
                .builder
                .open()
                .is_some_and(|open| open.first_trade_id == trade.venue_trade_id)
            {
                self.open_since = Some(seq);
            }
        }
        closed
    }
}

async fn publish(config: &Config, bus: &Bus, shutdown: &CancellationToken, batch: &Record) -> bool {
    let mut attempts = Retry::new(config.retry, shutdown, "publish bubbles");
    loop {
        match bus.publish_record(&config.namespace, batch).await {
            Ok(_) => {
                attempts.succeeded();
                return true;
            }
            Err(error) if attempts.failed(&error).await => {}
            Err(_) => return false,
        }
    }
}
