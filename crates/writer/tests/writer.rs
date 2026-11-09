use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::TryStreamExt;

use orderflow_bus::{Bus, StreamSpec};
use orderflow_collector::binance::{Frame, parse_frame};
use orderflow_contracts::{convert, v1};
use orderflow_domain::{
    Decimal, Gap, GapReason, Market, PriceGrouping, TickSize, TimeRange, Timeframe, Timestamp,
};
use orderflow_runtime::Backoff;
use orderflow_store::{Store, StoreConfig};
use orderflow_writer::{Config, run};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use venue_sim::FaultProxy;
use venue_sim::captures::{self, btc_perp as btc_capture};

const NATS_URL: &str = "nats://127.0.0.1:4222";

const CHAOS_CLICKHOUSE_URL: &str = "http://127.0.0.1:18123";
const CHAOS_CLICKHOUSE_CONTAINER: &str = "orderflow-clickhouse-chaos-1";

struct Harness {
    bus: Bus,
    spec: StreamSpec,
    store_config: StoreConfig,
    store: Store,
    subject_prefix: String,
}

impl Harness {
    async fn new(test: &str) -> Self {
        Self::with_store(test, StoreConfig::local()).await
    }

    async fn with_store(test: &str, server: StoreConfig) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let subject_prefix = format!("wtest.{test}.{nonce}");
        let spec = StreamSpec {
            name: format!("WTEST_{}_{nonce}", test.to_uppercase()),
            subjects: vec![format!("{subject_prefix}.>")],
        };
        let bus = Bus::connect(NATS_URL).await.unwrap();
        bus.ensure_stream(&spec).await.unwrap();
        let store_config = StoreConfig {
            database: format!("of_wtest_{test}_{}", std::process::id()),
            ..server
        };
        Store::connect(&store_config)
            .await
            .unwrap()
            .drop_database_for_tests()
            .await
            .unwrap();
        let store = Store::connect(&store_config).await.unwrap();
        Self {
            bus,
            spec,
            store_config,
            store,
            subject_prefix,
        }
    }

    fn config(&self, max_batch_trades: usize) -> Config {
        Config {
            stream: self.spec.clone(),
            max_batch_trades,
            linger: Duration::from_millis(20),
            retry: Backoff::new(Duration::from_millis(50), Duration::from_secs(1)),
        }
    }

    fn start(&self, config: Config) -> (CancellationToken, JoinHandle<anyhow::Result<()>>) {
        let shutdown = CancellationToken::new();
        let task = tokio::spawn({
            let (bus, store_config, shutdown) = (
                self.bus.clone(),
                self.store_config.clone(),
                shutdown.clone(),
            );
            async move {
                let store = Store::connect(&store_config).await?;
                run(&config, &bus, &store, shutdown).await
            }
        });
        (shutdown, task)
    }

    async fn publish_capture(&self) -> u64 {
        let mut last = 0;
        let subject = format!("{}.trade", self.subject_prefix);
        for frame in captures::btc_perp().frames() {
            let Ok(Frame::AggTrade { trade, .. }) = parse_frame(&frame.text) else {
                panic!("fixture frame is not an aggTrade");
            };
            let batch = convert::trade_batch_to_proto(&btc(), &[trade]).unwrap();
            last = self.publish(&subject, &batch).await;
        }
        last
    }

    async fn publish<M: prost::Message + prost::Name>(&self, subject: &str, message: &M) -> u64 {
        self.bus
            .publish(subject, None, message)
            .await
            .unwrap()
            .sequence
    }

    async fn wait_for_high_water_mark(&self, seq: u64) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let hwm = self.store.high_water_mark().await.unwrap_or(0);
            if hwm >= seq {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "stuck at {hwm} / {seq}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn capture_totals(&self) -> (Decimal, Decimal, u64) {
        let bars = self
            .store
            .footprint_bars(
                &btc(),
                everything(),
                Timeframe::from_secs(3_600).unwrap(),
                PriceGrouping::new(TickSize::new("0.1".parse().unwrap()).unwrap(), 1).unwrap(),
            )
            .await
            .unwrap();
        (
            bars.iter()
                .map(orderflow_domain::FootprintBar::buy_volume)
                .sum(),
            bars.iter()
                .map(orderflow_domain::FootprintBar::sell_volume)
                .sum(),
            bars.iter()
                .map(orderflow_domain::FootprintBar::trade_count)
                .sum(),
        )
    }

    async fn cleanup(self) {
        self.bus.delete_stream(&self.spec.name).await.unwrap();
        self.store.drop_database_for_tests().await.unwrap();
    }
}

fn btc() -> Market {
    "binance:perp:BTCUSDT".parse().unwrap()
}

fn everything() -> TimeRange {
    TimeRange::new(
        Timestamp::from_micros(0),
        Timestamp::from_micros(i64::MAX / 2),
    )
    .unwrap()
}

fn expected_totals() -> (Decimal, Decimal, u64) {
    (
        btc_capture::BUY_VOLUME.parse().unwrap(),
        btc_capture::SELL_VOLUME.parse().unwrap(),
        btc_capture::TRADES,
    )
}

async fn stop(shutdown: CancellationToken, task: JoinHandle<anyhow::Result<()>>) {
    shutdown.cancel();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn every_recorded_trade_lands_in_clickhouse_exactly_once() {
    let h = Harness::new("all").await;
    let last = h.publish_capture().await;

    let (shutdown, task) = h.start(h.config(1_000));
    h.wait_for_high_water_mark(last).await;
    stop(shutdown, task).await;

    assert_eq!(h.capture_totals().await, expected_totals());
    assert_eq!(
        h.store.recording_horizon(&btc()).await.unwrap(),
        Some(Timestamp::from_millis(btc_capture::FIRST_TRADE_MS))
    );

    let trades: Vec<_> = captures::btc_perp()
        .frames()
        .iter()
        .map(|f| match parse_frame(&f.text) {
            Ok(Frame::AggTrade { trade, .. }) => trade,
            other => panic!("unexpected frame {other:?}"),
        })
        .collect();
    let minute = Timeframe::from_secs(60).unwrap();
    for multiple in [1, 10, 250] {
        let grouping =
            PriceGrouping::new(TickSize::new("0.1".parse().unwrap()).unwrap(), multiple).unwrap();
        assert_eq!(
            h.store
                .footprint_bars(&btc(), everything(), minute, grouping)
                .await
                .unwrap(),
            orderflow_domain::footprint_bars(&trades, minute, grouping),
            "grouping x{multiple}"
        );
    }
    h.cleanup().await;
}

#[tokio::test]
async fn a_writer_crash_mid_replay_loses_and_duplicates_nothing() {
    let h = Harness::new("crash").await;
    let last = h.publish_capture().await;

    let (_, task) = h.start(h.config(200));
    h.wait_for_high_water_mark(last / 3).await;
    task.abort();
    let _ = task.await;
    let at_crash = h.store.high_water_mark().await.unwrap();
    assert!(at_crash < last, "the crash happened mid-replay");

    let (shutdown, task) = h.start(h.config(200));
    h.wait_for_high_water_mark(last).await;
    stop(shutdown, task).await;

    assert_eq!(h.capture_totals().await, expected_totals());
    h.cleanup().await;
}

#[tokio::test]
async fn gaps_are_stored_with_market_start_end_and_reason() {
    let h = Harness::new("gaps").await;
    let subject = format!("{}.gap", h.subject_prefix);
    h.publish(
        &subject,
        &v1::GapOpened {
            gap_id: "g1".into(),
            market: btc().to_string(),
            start_time_us: 1_000,
            reason: v1::GapReason::Disconnect.into(),
        },
    )
    .await;
    let last = h
        .publish(
            &subject,
            &v1::GapClosed {
                gap_id: "g1".into(),
                market: btc().to_string(),
                end_time_us: 2_000,
            },
        )
        .await;
    let trade = convert::trade_batch_to_proto(
        &btc(),
        &[orderflow_domain::Trade {
            venue_trade_id: 1,
            engine_time: Timestamp::from_micros(3_000),
            price: "1".parse().unwrap(),
            size: "1".parse().unwrap(),
            taker_side: orderflow_domain::TakerSide::Buy,
            taker_order_id: None,
        }],
    )
    .unwrap();
    let last = last.max(
        h.publish(&format!("{}.trade", h.subject_prefix), &trade)
            .await,
    );

    let (shutdown, task) = h.start(h.config(1_000));
    h.wait_for_high_water_mark(last).await;
    stop(shutdown, task).await;

    assert_eq!(
        h.store.gaps(&btc(), everything()).await.unwrap(),
        vec![
            Gap::closed(
                Timestamp::from_micros(1_000),
                Timestamp::from_micros(2_000),
                GapReason::Disconnect
            )
            .unwrap()
        ]
    );
    h.cleanup().await;
}

#[tokio::test]
async fn data_lost_from_the_stream_while_storage_was_down_becomes_a_storage_outage_gap() {
    let h = Harness::new("outage").await;
    let subject = format!("{}.trade", h.subject_prefix);
    let trade_at = |id: u64, ms: i64| {
        convert::trade_batch_to_proto(
            &btc(),
            &[orderflow_domain::Trade {
                venue_trade_id: id,
                engine_time: Timestamp::from_millis(ms),
                price: "100".parse().unwrap(),
                size: "1".parse().unwrap(),
                taker_side: orderflow_domain::TakerSide::Buy,
                taker_order_id: None,
            }],
        )
        .unwrap()
    };

    let first = h.publish(&subject, &trade_at(1, 1_000)).await;
    let (shutdown, task) = h.start(h.config(1_000));
    h.wait_for_high_water_mark(first).await;
    stop(shutdown, task).await;

    h.publish(&subject, &trade_at(2, 2_000)).await;
    h.bus
        .jetstream()
        .get_stream(&h.spec.name)
        .await
        .unwrap()
        .purge()
        .await
        .unwrap();
    let after = h.publish(&subject, &trade_at(3, 3_000)).await;

    let (shutdown, task) = h.start(h.config(1_000));
    h.wait_for_high_water_mark(after).await;
    stop(shutdown, task).await;

    assert_eq!(
        h.store.gaps(&btc(), everything()).await.unwrap(),
        vec![
            Gap::closed(
                Timestamp::from_millis(1_000),
                Timestamp::from_millis(3_000),
                GapReason::StorageOutage
            )
            .unwrap()
        ]
    );
    h.cleanup().await;
}

#[tokio::test]
#[ignore = "restarts a ClickHouse container; run by `cargo xtask ci`"]
async fn a_clickhouse_restart_mid_replay_loses_nothing() {
    let h = Harness::with_store(
        "chrestart",
        StoreConfig {
            url: CHAOS_CLICKHOUSE_URL.into(),
            ..StoreConfig::local()
        },
    )
    .await;
    let last = h.publish_capture().await;

    let (shutdown, task) = h.start(h.config(200));
    h.wait_for_high_water_mark(last / 4).await;
    let status = std::process::Command::new("docker")
        .args(["restart", CHAOS_CLICKHOUSE_CONTAINER])
        .status()
        .unwrap();
    assert!(status.success());
    h.wait_for_high_water_mark(last).await;
    stop(shutdown, task).await;

    assert_eq!(h.capture_totals().await, expected_totals());
    h.cleanup().await;
}

#[tokio::test]
async fn a_consumer_lost_while_disconnected_resumes_from_clickhouse_without_loss_or_duplicates() {
    let h = Harness::new("consumerloss").await;
    let last = h.publish_capture().await;
    let nats = FaultProxy::start(NATS_URL.trim_start_matches("nats://").to_owned())
        .await
        .unwrap();
    let writer_bus = Bus::connect(&format!("nats://{}", nats.addr()))
        .await
        .unwrap();

    let shutdown = CancellationToken::new();
    let task = tokio::spawn({
        let (config, store_config, shutdown) =
            (h.config(200), h.store_config.clone(), shutdown.clone());
        async move {
            let store = Store::connect(&store_config).await?;
            run(&config, &writer_bus, &store, shutdown).await
        }
    });
    h.wait_for_high_water_mark(last).await;

    nats.cut();
    let stream = h.bus.jetstream().get_stream(&h.spec.name).await.unwrap();
    let names: Vec<String> = stream.consumer_names().try_collect().await.unwrap();
    assert!(!names.is_empty(), "the writer has a consumer");
    for name in names {
        stream.delete_consumer(&name).await.unwrap();
    }
    nats.restore();

    let last = h.publish_capture().await;
    h.wait_for_high_water_mark(last).await;
    assert!(
        !task.is_finished(),
        "the writer stopped instead of resuming"
    );
    stop(shutdown, task).await;

    let (buy, sell, trades) = expected_totals();
    assert_eq!(
        h.capture_totals().await,
        (buy * Decimal::TWO, sell * Decimal::TWO, trades * 2)
    );
    h.cleanup().await;
}
