use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use orderflow_collector::checkpoint::{Checkpoint, Checkpoints, MemoryCheckpoints};
use orderflow_collector::sink::{MemorySink, Record, Sink};
use orderflow_collector::{Backoff, Config, run};
use orderflow_domain::{Decimal, GapReason, Market, TakerSide, Timestamp};
use tokio_util::sync::CancellationToken;
use venue_sim::binance::AggTrades;
use venue_sim::{FakeVenue, Fault, Fixture, Frame, Script};

fn btc() -> Market {
    "binance:perp:BTCUSDT".parse().unwrap()
}

fn ms(t: i64) -> Timestamp {
    Timestamp::from_millis(t)
}

fn config(venue: &FakeVenue) -> Config {
    Config {
        venue_base_url: venue.url(),
        markets: vec![btc()],
        stall_timeout: Duration::from_secs(5),
        reconnect_min: Duration::from_millis(10),
        reconnect_max: Duration::from_millis(50),
        checkpoint_interval: Duration::from_millis(10),
        retry: Backoff::new(Duration::from_millis(5), Duration::from_millis(20)),
    }
}

async fn collect_until(
    config: Config,
    checkpoints: &MemoryCheckpoints,
    done: impl Fn(&[Record]) -> bool,
) -> Vec<Record> {
    let records = MemorySink::default();
    let (published, result) =
        run_until(config, records.clone(), checkpoints.clone(), &records, done).await;
    result.expect("collector failed");
    published
}

async fn run_until<S, C>(
    config: Config,
    mut sink: S,
    checkpoints: C,
    records: &MemorySink,
    done: impl Fn(&[Record]) -> bool,
) -> (Vec<Record>, anyhow::Result<()>)
where
    S: Sink + Send + 'static,
    C: Checkpoints + Send + Sync + 'static,
{
    let shutdown = CancellationToken::new();
    let task = tokio::spawn({
        let shutdown = shutdown.clone();
        async move { run(&config, &mut sink, &checkpoints, shutdown).await }
    });

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !done(&records.records()) {
        if task.is_finished() {
            let result = task.await.unwrap();
            return (
                records.records(),
                result.and(Err(anyhow::anyhow!("stopped early"))),
            );
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out; records so far: {:#?}",
            records.records()
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    shutdown.cancel();
    let result = task.await.unwrap();
    (records.records(), result)
}

#[derive(Debug, Clone)]
struct Flaky<T> {
    inner: T,
    failures: Arc<AtomicU32>,
}

impl<T> Flaky<T> {
    fn new(inner: T, failures: u32) -> Self {
        Self {
            inner,
            failures: Arc::new(AtomicU32::new(failures)),
        }
    }

    fn fail(&self) -> anyhow::Result<()> {
        if self
            .failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            anyhow::bail!("timed out: didn't receive ack in time");
        }
        Ok(())
    }
}

impl<S: Sink + Send> Sink for Flaky<S> {
    async fn publish(&mut self, record: &Record) -> anyhow::Result<()> {
        self.fail()?;
        self.inner.publish(record).await
    }
}

impl<C: Checkpoints + Sync> Checkpoints for Flaky<C> {
    async fn load(&self, market: &Market) -> anyhow::Result<Option<Checkpoint>> {
        self.inner.load(market).await
    }

    async fn save(&self, market: &Market, checkpoint: &Checkpoint) -> anyhow::Result<()> {
        self.fail()?;
        self.inner.save(market, checkpoint).await
    }
}

fn trades(records: &[Record]) -> Vec<(u64, TakerSide, Decimal, Decimal, Timestamp)> {
    records
        .iter()
        .filter_map(|r| match r {
            Record::Trades { market, trades } => {
                assert_eq!(market, &btc());
                Some(trades)
            }
            _ => None,
        })
        .flatten()
        .map(|t| {
            (
                t.venue_trade_id,
                t.taker_side,
                t.price,
                t.size,
                t.engine_time,
            )
        })
        .collect()
}

fn trade_ids(records: &[Record]) -> Vec<u64> {
    trades(records).into_iter().map(|t| t.0).collect()
}

fn gaps(records: &[Record]) -> Vec<(Timestamp, Option<Timestamp>, GapReason)> {
    let mut out = Vec::new();
    for record in records {
        match record {
            Record::GapOpened {
                gap_id,
                start,
                reason,
                ..
            } => out.push((gap_id.clone(), *start, None, *reason)),
            Record::GapClosed { gap_id, end, .. } => {
                let open = out
                    .iter_mut()
                    .find(|g| &g.0 == gap_id)
                    .expect("GapClosed matches an earlier GapOpened");
                assert!(open.2.is_none(), "gap closed twice");
                open.2 = Some(*end);
            }
            Record::Trades { .. } | Record::Bubbles { .. } => {}
        }
    }
    out.into_iter().map(|(_, s, e, r)| (s, e, r)).collect()
}

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

#[tokio::test]
async fn trades_are_published_in_venue_order_with_taker_side_from_the_maker_flag() {
    let fixture = AggTrades::new("BTCUSDT")
        .buy(1_000, "75191.30", "0.086")
        .sell(1_001, "75191.20", "1.5")
        .build();
    let venue = FakeVenue::start(Script::new(fixture)).await.unwrap();

    let records = collect_until(config(&venue), &MemoryCheckpoints::default(), |r| {
        trades(r).len() == 2
    })
    .await;

    assert_eq!(
        trades(&records),
        vec![
            (1_000, TakerSide::Buy, d("75191.30"), d("0.086"), ms(1_000)),
            (1_001, TakerSide::Sell, d("75191.20"), d("1.5"), ms(1_001)),
        ]
    );
    assert_eq!(gaps(&records), vec![]);
    assert_eq!(
        venue.requested_paths(),
        vec!["/stream?streams=btcusdt@aggTrade"]
    );
}

#[tokio::test]
async fn a_disconnect_records_one_gap_from_the_last_trade_before_to_the_first_after() {
    let fixture = (0..6)
        .fold(AggTrades::new("BTCUSDT"), |b, i| {
            b.buy(1_000 + i * 100, "100", "1")
        })
        .build();
    let script = Script::new(fixture).with_fault(Fault::Disconnect {
        after_frames: 3,
        lost_frames: 1,
    });
    let venue = FakeVenue::start(script).await.unwrap();

    let records = collect_until(config(&venue), &MemoryCheckpoints::default(), |r| {
        trade_ids(r).last() == Some(&1_005)
    })
    .await;

    assert_eq!(trade_ids(&records), vec![1_000, 1_001, 1_002, 1_004, 1_005]);
    // The id jump 1002 → 1004 is inside the disconnect Gap, not a second Gap.
    assert_eq!(
        gaps(&records),
        vec![(ms(1_200), Some(ms(1_400)), GapReason::Disconnect)]
    );
    assert_eq!(venue.connections(), 2);
}

#[tokio::test]
async fn a_skipped_aggregate_id_records_a_sequence_break_gap() {
    let fixture = (0..5)
        .fold(AggTrades::new("BTCUSDT"), |b, i| {
            b.sell(2_000 + i * 10, "100", "1")
        })
        .build();
    let script = Script::new(fixture).with_fault(Fault::Drop {
        at_frame: 2,
        count: 1,
    });
    let venue = FakeVenue::start(script).await.unwrap();

    let records = collect_until(config(&venue), &MemoryCheckpoints::default(), |r| {
        trade_ids(r).len() == 4
    })
    .await;

    assert_eq!(trade_ids(&records), vec![1_000, 1_001, 1_003, 1_004]);
    assert_eq!(
        gaps(&records),
        vec![(ms(2_010), Some(ms(2_030)), GapReason::SequenceBreak)]
    );
    assert_eq!(venue.connections(), 1);
}

#[tokio::test]
async fn a_restart_records_recorder_downtime_from_the_last_checkpoint() {
    let checkpoints = MemoryCheckpoints::default();
    let first = AggTrades::new("BTCUSDT")
        .buy(5_000, "100", "1")
        .buy(5_100, "100", "1")
        .build();
    let venue = FakeVenue::start(Script::new(first)).await.unwrap();
    let before = collect_until(config(&venue), &checkpoints, |r| trades(r).len() == 2).await;
    assert_eq!(gaps(&before), vec![]);

    // The collector was down while ids 1002..1009 traded.
    let later = AggTrades::new("BTCUSDT")
        .skip_ids(10)
        .buy(9_000, "101", "1")
        .build();
    let venue = FakeVenue::start(Script::new(later)).await.unwrap();
    let after = collect_until(config(&venue), &checkpoints, |r| trades(r).len() == 1).await;

    assert_eq!(trade_ids(&after), vec![1_010]);
    assert_eq!(
        gaps(&after),
        vec![(ms(5_100), Some(ms(9_000)), GapReason::RecorderDowntime)]
    );
}

#[tokio::test]
async fn a_silent_stream_is_treated_as_a_disconnect() {
    let fixture = AggTrades::new("BTCUSDT").buy(1_000, "100", "1").build();
    let venue = FakeVenue::start(Script::new(fixture)).await.unwrap();
    let config = Config {
        stall_timeout: Duration::from_millis(150),
        ..config(&venue)
    };

    // Wait for the stall Gap and a reconnect; the reconnected stream stays silent.
    let records = collect_until(config, &MemoryCheckpoints::default(), |r| {
        !gaps(r).is_empty() && venue.connections() >= 2
    })
    .await;

    // One Gap, still open: repeated stalls do not stack Gaps.
    assert_eq!(
        gaps(&records),
        vec![(ms(1_000), None, GapReason::Disconnect)]
    );
}

#[tokio::test]
async fn duplicate_and_non_trade_frames_are_ignored() {
    let built = AggTrades::new("BTCUSDT")
        .buy(1_000, "100", "1")
        .buy(1_001, "100", "2")
        .build();
    let [a, b] = built.frames() else {
        unreachable!()
    };
    let fixture = Fixture::new(vec![
        Frame {
            recv_us: 0,
            text: r#"{"result":null,"id":1}"#.into(),
        },
        a.clone(),
        a.clone(), // replayed after a Venue hiccup
        b.clone(),
    ]);
    let venue = FakeVenue::start(Script::new(fixture)).await.unwrap();

    let records = collect_until(config(&venue), &MemoryCheckpoints::default(), |r| {
        trade_ids(r).last() == Some(&1_001)
    })
    .await;

    assert_eq!(trade_ids(&records), vec![1_000, 1_001]);
    assert_eq!(gaps(&records), vec![]);
}

#[tokio::test]
async fn only_binance_perp_markets_are_accepted() {
    let venue = FakeVenue::start(Script::new(Fixture::default()))
        .await
        .unwrap();
    let config = Config {
        markets: vec!["hyperliquid:perp:BTC".parse().unwrap()],
        ..config(&venue)
    };

    let result = run(
        &config,
        &mut MemorySink::default(),
        &MemoryCheckpoints::default(),
        CancellationToken::new(),
    )
    .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn publishes_and_checkpoint_saves_that_fail_transiently_are_retried_without_losing_trades() {
    let fixture = (0..6)
        .fold(AggTrades::new("BTCUSDT"), |b, i| {
            b.buy(3_000 + i * 10, "100", "1")
        })
        .build();
    let venue = FakeVenue::start(Script::new(fixture)).await.unwrap();
    let records = MemorySink::default();
    let checkpoints = MemoryCheckpoints::default();

    let (published, result) = run_until(
        config(&venue),
        Flaky::new(records.clone(), 3),
        Flaky::new(checkpoints.clone(), 3),
        &records,
        // A saved checkpoint means the failing saves were retried through.
        |r| trades(r).len() == 6 && checkpoints.get(&btc()).is_some(),
    )
    .await;

    result.expect("transient failures must not stop the collector");
    assert_eq!(
        trade_ids(&published),
        vec![1_000, 1_001, 1_002, 1_003, 1_004, 1_005]
    );
    assert_eq!(gaps(&published), vec![]);
    let saved = checkpoints.load(&btc()).await.unwrap().unwrap();
    assert_eq!(saved.last_trade_id, 1_005);
}

#[tokio::test]
async fn shutting_down_during_a_publish_outage_never_checkpoints_unpublished_trades() {
    let fixture = AggTrades::new("BTCUSDT")
        .buy(4_000, "100", "1")
        .buy(4_010, "100", "1")
        .build();
    let venue = FakeVenue::start(Script::new(fixture)).await.unwrap();
    let records = MemorySink::default();
    let checkpoints = MemoryCheckpoints::default();
    let sink = Flaky::new(records.clone(), u32::MAX);

    let shutdown = CancellationToken::new();
    let task = tokio::spawn({
        let (config, mut sink, checkpoints, shutdown) = (
            config(&venue),
            sink.clone(),
            checkpoints.clone(),
            shutdown.clone(),
        );
        async move { run(&config, &mut sink, &checkpoints, shutdown).await }
    });
    // Shut down once the collector has failed to publish a few times.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while sink.failures.load(Ordering::SeqCst) > u32::MAX - 3 && !task.is_finished() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no publish attempts"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    shutdown.cancel();
    let result = task.await.unwrap();

    assert!(
        result.is_err(),
        "unpublished Trades must not end in a clean exit"
    );
    assert_eq!(records.records(), vec![]);
    let saved = checkpoints.load(&btc()).await.unwrap();
    assert!(
        saved.is_none(),
        "checkpoint advanced past unpublished Trades: {saved:?}"
    );
}
