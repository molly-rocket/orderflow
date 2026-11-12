use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use orderflow_book_engine::{Config, run};
use orderflow_bus::{Bus, decode};
use orderflow_contracts::book::{BookState, book_state_from_proto};
use orderflow_contracts::subjects::Namespace;
use orderflow_contracts::v1;
use orderflow_domain::{BookStatus, Decimal, Level, MarketSpec};
use orderflow_runtime::Backoff;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use venue_sim::binance::{DepthDiffs, depth_snapshot};
use venue_sim::{FakeRest, FakeVenue, Fault, Pacing, Script};

const NATS_URL: &str = "nats://127.0.0.1:4222";
const BIDS: &[(&str, &str)] = &[("100.0", "5"), ("99.5", "4"), ("99.0", "3")];
const ASKS: &[(&str, &str)] = &[("100.1", "5"), ("100.5", "4"), ("101.0", "3")];

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn lvl(price: &str, size: &str) -> Level {
    Level {
        price: d(price),
        size: d(size),
    }
}

fn spec() -> MarketSpec {
    "binance:perp:BTCUSDT=0.1".parse().unwrap()
}

fn namespace(test: &str) -> Namespace {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    Namespace::new(&format!("bk_{test}_{nonce}")).unwrap()
}

struct Harness {
    _venue: FakeVenue,
    rest: FakeRest,
    books: async_nats::Subscriber,
    heatmaps: async_nats::Subscriber,
    shutdown: CancellationToken,
    task: JoinHandle<anyhow::Result<()>>,
}

impl Harness {
    async fn start(test: &str, script: Script, snapshots: Vec<(u16, String)>) -> Self {
        Self::start_with(test, script, snapshots, Duration::from_secs(30)).await
    }

    async fn start_with(
        test: &str,
        script: Script,
        snapshots: Vec<(u16, String)>,
        stall_timeout: Duration,
    ) -> Self {
        let venue = FakeVenue::start(script).await.unwrap();
        let rest = FakeRest::start(snapshots).await.unwrap();
        let bus = Bus::connect(NATS_URL).await.unwrap();
        let namespace = namespace(test);
        let market = spec().market;
        let books = bus
            .client()
            .subscribe(namespace.book(&market))
            .await
            .unwrap();
        let heatmaps = bus
            .client()
            .subscribe(namespace.heatmap(&market))
            .await
            .unwrap();
        let config = Config {
            namespace,
            markets: vec![spec()],
            ws_url: venue.url(),
            rest_url: rest.url(),
            stall_timeout,
            snapshot_timeout: Duration::from_millis(500),
            retry: Backoff::new(Duration::from_millis(50), Duration::from_millis(200)),
            max_buffer: 1000,
        };
        let shutdown = CancellationToken::new();
        let task = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { run(&config, &bus, shutdown).await }
        });
        Self {
            _venue: venue,
            rest,
            books,
            heatmaps,
            shutdown,
            task,
        }
    }

    async fn next_book(&mut self) -> BookState {
        let message = tokio::time::timeout(Duration::from_secs(5), self.books.next())
            .await
            .expect("a book state within 5 s")
            .expect("subscription open");
        let proto: v1::BookState = decode(message.headers.as_ref(), &message.payload).unwrap();
        book_state_from_proto(proto).unwrap()
    }

    async fn book_until(&mut self, done: impl Fn(&BookState) -> bool) -> Vec<BookState> {
        let mut seen = Vec::new();
        loop {
            let state = self.next_book().await;
            let finished = done(&state);
            seen.push(state);
            if finished {
                return seen;
            }
        }
    }

    async fn stop(self) {
        self.shutdown.cancel();
        self.task.await.unwrap().unwrap();
    }
}

fn live_with_bid(price: &str, size: &str) -> impl Fn(&BookState) -> bool {
    let wanted = lvl(price, size);
    move |s| s.status == BookStatus::Live && s.bids.first() == Some(&wanted)
}

#[tokio::test]
async fn publishes_a_live_book_matching_snapshot_plus_diffs() {
    let fixture = DepthDiffs::new("BTCUSDT")
        .diff(1, 4, 0, &[("99.0", "9")], &[])
        .diff(5, 8, 4, &[("99.5", "8")], &[("100.5", "0")])
        .diff(9, 12, 8, &[], &[("100.9", "2")])
        .build();
    let mut h = Harness::start(
        "live",
        Script::new(fixture),
        vec![(200, depth_snapshot(6, BIDS, ASKS))],
    )
    .await;

    let states = h
        .book_until(|s| {
            s.status == BookStatus::Live && s.asks.iter().any(|l| l.price == d("100.9"))
        })
        .await;

    let live = states.last().unwrap();
    assert_eq!(
        live.bids,
        vec![lvl("100.0", "5"), lvl("99.5", "8"), lvl("99.0", "3")]
    );
    assert_eq!(
        live.asks,
        vec![lvl("100.1", "5"), lvl("100.9", "2"), lvl("101.0", "3")]
    );
    assert_eq!(live.known, Some((d("99.0"), d("101.0"))));
    assert_eq!(live.market, spec().market);
    assert_eq!(
        h.rest.requested_paths(),
        vec!["/fapi/v1/depth?symbol=BTCUSDT&limit=1000"]
    );
    h.stop().await;
}

#[tokio::test]
async fn a_dropped_diff_resyncs_and_recovers() {
    let fixture = DepthDiffs::new("BTCUSDT")
        .diff(1, 4, 0, &[], &[])
        .diff(5, 8, 4, &[("99.5", "8")], &[])
        .diff(9, 12, 8, &[("99.0", "1")], &[])
        .diff(13, 16, 12, &[("200.0", "3")], &[])
        .build();
    let script = Script::new(fixture)
        .with_pacing(Pacing::Realtime { speed: 1.0 })
        .with_fault(Fault::Drop {
            at_frame: 2,
            count: 1,
        });
    let mut h = Harness::start(
        "drop",
        script,
        vec![
            (200, depth_snapshot(6, BIDS, ASKS)),
            (
                200,
                depth_snapshot(14, &[("200.0", "1")], &[("200.1", "1")]),
            ),
        ],
    )
    .await;

    h.book_until(live_with_bid("100.0", "5")).await;
    let states = h.book_until(live_with_bid("200.0", "3")).await;

    let resyncing = states
        .iter()
        .find(|s| s.status == BookStatus::Resyncing)
        .expect("a resyncing state between the two live books");
    assert!(resyncing.bids.is_empty() && resyncing.asks.is_empty() && resyncing.known.is_none());
    assert_eq!(h.rest.requested_paths().len(), 2);
    h.stop().await;
}

#[tokio::test]
async fn a_failed_snapshot_backs_off_and_stays_resyncing() {
    let fixture = DepthDiffs::new("BTCUSDT")
        .spaced_ms(600)
        .diff(1, 4, 0, &[], &[])
        .diff(5, 8, 4, &[("99.5", "8")], &[])
        .build();
    let script = Script::new(fixture).with_pacing(Pacing::Realtime { speed: 1.0 });
    let mut h = Harness::start(
        "fail",
        script,
        vec![(429, "{}".into()), (200, depth_snapshot(6, BIDS, ASKS))],
    )
    .await;

    let states = h.book_until(live_with_bid("100.0", "5")).await;

    let before_live = &states[..states.len() - 1];
    assert!(before_live.iter().all(|s| s.status != BookStatus::Live));
    assert!(
        before_live
            .iter()
            .any(|s| s.status == BookStatus::Resyncing)
    );
    assert_eq!(h.rest.requested_paths().len(), 2);
    h.stop().await;
}

#[tokio::test]
async fn a_hanging_snapshot_request_times_out_and_is_retried() {
    let fixture = DepthDiffs::new("BTCUSDT")
        .spaced_ms(1_500)
        .diff(1, 4, 0, &[], &[])
        .diff(5, 8, 4, &[("99.5", "8")], &[])
        .build();
    let script = Script::new(fixture).with_pacing(Pacing::Realtime { speed: 1.0 });
    let mut h = Harness::start(
        "hang",
        script,
        vec![(0, String::new()), (200, depth_snapshot(6, BIDS, ASKS))],
    )
    .await;

    h.book_until(live_with_bid("100.0", "5")).await;

    assert_eq!(h.rest.requested_paths().len(), 2);
    h.stop().await;
}

#[tokio::test]
async fn a_silent_venue_is_treated_as_disconnected() {
    let fixture = DepthDiffs::new("BTCUSDT")
        .diff(1, 4, 0, &[], &[])
        .diff(5, 8, 4, &[], &[])
        .build();
    let mut h = Harness::start_with(
        "silent",
        Script::new(fixture),
        vec![(200, depth_snapshot(6, BIDS, ASKS))],
        Duration::from_secs(1),
    )
    .await;
    h.book_until(live_with_bid("100.0", "5")).await;

    let started = tokio::time::Instant::now();
    h.book_until(|s| s.status == BookStatus::Disconnected).await;

    assert!(started.elapsed() < Duration::from_secs(4));
    let mut heatmap_disconnected = false;
    while let Ok(Some(message)) =
        tokio::time::timeout(Duration::from_millis(1_500), h.heatmaps.next()).await
    {
        let proto: v1::BookState = decode(message.headers.as_ref(), &message.payload).unwrap();
        if book_state_from_proto(proto).unwrap().status == BookStatus::Disconnected {
            heatmap_disconnected = true;
            break;
        }
    }
    assert!(heatmap_disconnected, "no disconnected heatmap frame");
    h.stop().await;
}

#[tokio::test]
async fn heatmap_frames_arrive_once_a_second() {
    let fixture = DepthDiffs::new("BTCUSDT")
        .diff(1, 4, 0, &[], &[])
        .diff(5, 8, 4, &[], &[])
        .build();
    let mut h = Harness::start(
        "heat",
        Script::new(fixture),
        vec![(200, depth_snapshot(6, BIDS, ASKS))],
    )
    .await;
    h.book_until(live_with_bid("100.0", "5")).await;

    let mut frames: Vec<BookState> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(2_600);
    while let Ok(Some(message)) = tokio::time::timeout_at(deadline, h.heatmaps.next()).await {
        let proto: v1::BookState = decode(message.headers.as_ref(), &message.payload).unwrap();
        frames.push(book_state_from_proto(proto).unwrap());
    }

    let live: Vec<&BookState> = frames
        .iter()
        .filter(|f| f.status == BookStatus::Live)
        .collect();
    assert!(live.len() >= 2, "got {} live frames", live.len());
    let gaps: Vec<i64> = live
        .windows(2)
        .map(|pair| pair[1].time.as_micros() - pair[0].time.as_micros())
        .collect();
    assert!(
        gaps.iter().all(|gap| *gap <= 1_300_000),
        "frame gaps {gaps:?}"
    );
    assert!(
        gaps.iter().any(|gap| *gap >= 700_000),
        "frame gaps {gaps:?}"
    );
    assert!(live.iter().all(|f| f.bids[0] == lvl("100.0", "5")));
    h.stop().await;
}

#[tokio::test]
async fn a_venue_disconnect_publishes_disconnected_then_recovers() {
    let fixture = DepthDiffs::new("BTCUSDT")
        .diff(1, 4, 0, &[], &[])
        .diff(5, 8, 4, &[], &[])
        .diff(9, 12, 8, &[("100.0", "7")], &[])
        .diff(13, 16, 12, &[], &[])
        .build();
    let script = Script::new(fixture)
        .with_pacing(Pacing::Realtime { speed: 1.0 })
        .with_fault(Fault::Disconnect {
            after_frames: 2,
            lost_frames: 0,
        });
    let mut h = Harness::start(
        "disc",
        script,
        vec![
            (200, depth_snapshot(6, BIDS, ASKS)),
            (200, depth_snapshot(10, BIDS, ASKS)),
        ],
    )
    .await;

    h.book_until(live_with_bid("100.0", "5")).await;
    let states = h.book_until(live_with_bid("100.0", "7")).await;

    assert!(
        states
            .iter()
            .any(|s| s.status == BookStatus::Disconnected && s.bids.is_empty())
    );
    h.stop().await;
}
