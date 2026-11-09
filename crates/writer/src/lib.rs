use std::collections::HashMap;
use std::time::Duration;

use anyhow::Context;
use async_nats::jetstream::consumer::{DeliverPolicy, pull};
use futures_util::StreamExt;
use orderflow_bus::{Bus, StreamSpec, decode_record};
use orderflow_contracts::record::Record;
use orderflow_domain::{GapReason, Market, Timestamp};
use orderflow_runtime::{Backoff, retry};
use orderflow_store::{GapEvent, Store, TradeBatchRecord, WriteBatch};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct Config {
    pub stream: StreamSpec,
    pub max_batch_trades: usize,
    pub linger: Duration,
    pub retry: Backoff,
}

impl Config {
    pub fn for_namespace(namespace: &orderflow_contracts::subjects::Namespace) -> Self {
        Self {
            stream: StreamSpec::record(namespace),
            max_batch_trades: 50_000,
            linger: Duration::from_millis(200),
            retry: Backoff::new(Duration::from_millis(200), Duration::from_secs(10)),
        }
    }
}

pub async fn run(
    config: &Config,
    bus: &Bus,
    store: &Store,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    let stream = bus.ensure_stream(&config.stream).await?;
    let mut outage = Outage::default();
    let mut pending = WriteBatch::default();

    loop {
        let Some(high_water_mark) = retry(config.retry, &shutdown, "read high-water mark", || {
            store.high_water_mark()
        })
        .await
        else {
            return Ok(());
        };
        if !record_expired_messages(
            config,
            store,
            &stream,
            &shutdown,
            high_water_mark,
            &mut outage,
        )
        .await
        {
            return Ok(());
        }
        let Some(messages) = retry(config.retry, &shutdown, "open the record stream", || {
            open(&stream, high_water_mark + 1)
        })
        .await
        else {
            return Ok(());
        };
        tracing::info!(stream = %config.stream.name, from = high_water_mark + 1, "writing");

        match consume(
            config,
            store,
            &shutdown,
            messages,
            &mut outage,
            &mut pending,
        )
        .await
        {
            Consumed::Shutdown => break,
            Consumed::Interrupted(error) => {
                tracing::warn!(error = %format!("{error:#}"), "record stream interrupted; resuming from ClickHouse");
                if !flush(config, store, &shutdown, &mut pending).await {
                    return Ok(());
                }
            }
            Consumed::Stopped => return Ok(()),
        }
    }
    let _ = tokio::time::timeout(config.retry.max, store.write(&pending)).await;
    Ok(())
}

async fn open(
    stream: &async_nats::jetstream::stream::Stream,
    start_sequence: u64,
) -> anyhow::Result<pull::Ordered> {
    let consumer = stream
        .create_consumer(pull::OrderedConfig {
            deliver_policy: DeliverPolicy::ByStartSequence { start_sequence },
            ..Default::default()
        })
        .await
        .context("creating ordered consumer")?;
    Ok(consumer.messages().await?)
}

async fn record_expired_messages(
    config: &Config,
    store: &Store,
    stream: &async_nats::jetstream::stream::Stream,
    shutdown: &CancellationToken,
    high_water_mark: u64,
    outage: &mut Outage,
) -> bool {
    let Some(info) = retry(config.retry, shutdown, "read stream info", || {
        stream.get_info()
    })
    .await
    else {
        return false;
    };
    let first_available = info.state.first_sequence;
    if high_water_mark == 0 || first_available <= high_water_mark + 1 {
        return true;
    }
    tracing::error!(
        high_water_mark,
        first_available,
        "messages expired from the stream before they were stored; recording a StorageOutage gap"
    );
    let Some(latest) = retry(config.retry, shutdown, "read latest trades", || {
        store.latest_trades()
    })
    .await
    else {
        return false;
    };
    let batch = outage.open(&latest);
    retry(config.retry, shutdown, "write outage gaps", || {
        store.write(&batch)
    })
    .await
    .is_some()
}

enum Consumed {
    Shutdown,
    Interrupted(anyhow::Error),
    Stopped,
}

async fn consume(
    config: &Config,
    store: &Store,
    shutdown: &CancellationToken,
    mut messages: pull::Ordered,
    outage: &mut Outage,
    pending: &mut WriteBatch,
) -> Consumed {
    let mut flush_at: Option<Instant> = None;
    loop {
        let linger = async {
            match flush_at {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            () = shutdown.cancelled() => return Consumed::Shutdown,
            () = linger => {
                if !flush(config, store, shutdown, pending).await {
                    return Consumed::Stopped;
                }
                flush_at = None;
            }
            next = messages.next() => {
                let message = match next {
                    Some(Ok(message)) => message,
                    Some(Err(error)) => return Consumed::Interrupted(error.into()),
                    None => return Consumed::Interrupted(anyhow::anyhow!("record stream ended")),
                };
                let seq = match message.info() {
                    Ok(info) => info.stream_sequence,
                    Err(error) => return Consumed::Interrupted(anyhow::anyhow!(error)),
                };
                add(pending, outage, message.headers.as_ref(), &message.payload, seq);
                flush_at.get_or_insert_with(|| Instant::now() + config.linger);
                if pending.trade_count() >= config.max_batch_trades {
                    if !flush(config, store, shutdown, pending).await {
                        return Consumed::Stopped;
                    }
                    flush_at = None;
                }
            }
        }
    }
}

fn add(
    pending: &mut WriteBatch,
    outage: &mut Outage,
    headers: Option<&async_nats::HeaderMap>,
    payload: &[u8],
    seq: u64,
) {
    match decode_record(headers, payload) {
        Ok(Record::Trades { market, trades }) => {
            if let (Some(first), Some(close)) = (trades.first(), outage.close(&market)) {
                pending.gap_events.push((close.at(first.engine_time), seq));
            }
            pending.trade_batches.push(TradeBatchRecord {
                market,
                trades,
                stream_seq: seq,
            });
        }
        Ok(Record::GapOpened {
            gap_id,
            market,
            start,
            reason,
        }) => pending.gap_events.push((
            GapEvent::Opened {
                market,
                gap_id,
                start,
                reason,
            },
            seq,
        )),
        Ok(Record::GapClosed {
            gap_id,
            market,
            end,
        }) => pending.gap_events.push((
            GapEvent::Closed {
                market,
                gap_id,
                end,
            },
            seq,
        )),
        Err(error) => tracing::error!(seq, %error, "skipping undecodable record"),
    }
}

async fn flush(
    config: &Config,
    store: &Store,
    shutdown: &CancellationToken,
    pending: &mut WriteBatch,
) -> bool {
    if pending.is_empty() {
        return true;
    }
    let trades = pending.trade_count();
    if retry(config.retry, shutdown, "write batch", || {
        store.write(pending)
    })
    .await
    .is_none()
    {
        return false;
    }
    tracing::debug!(trades, gap_events = pending.gap_events.len(), "flushed");
    *pending = WriteBatch::default();
    true
}

#[derive(Debug, Default)]
struct Outage {
    open: HashMap<Market, String>,
}

struct PendingClose {
    market: Market,
    gap_id: String,
}

impl PendingClose {
    fn at(self, end: Timestamp) -> GapEvent {
        GapEvent::Closed {
            market: self.market,
            gap_id: self.gap_id,
            end,
        }
    }
}

impl Outage {
    fn open(&mut self, latest: &[orderflow_store::LatestTrade]) -> WriteBatch {
        let mut batch = WriteBatch::default();
        for trade in latest {
            let gap_id = format!("{}/{}/StorageOutage", trade.market, trade.venue_trade_id);
            batch.gap_events.push((
                GapEvent::Opened {
                    market: trade.market.clone(),
                    gap_id: gap_id.clone(),
                    start: trade.engine_time,
                    reason: GapReason::StorageOutage,
                },
                0,
            ));
            self.open.insert(trade.market.clone(), gap_id);
        }
        batch
    }

    fn close(&mut self, market: &Market) -> Option<PendingClose> {
        self.open.remove(market).map(|gap_id| PendingClose {
            market: market.clone(),
            gap_id,
        })
    }
}
