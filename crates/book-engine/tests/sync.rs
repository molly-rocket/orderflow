use orderflow_book_engine::binance::{DepthDiff, Snapshot, parse_frame, parse_snapshot};
use orderflow_book_engine::sync::{Action, BookSync};
use orderflow_domain::{BookSide, BookStatus, Decimal, Level, Timestamp};

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn lvl(price: &str, size: &str) -> Level {
    Level {
        price: d(price),
        size: d(size),
    }
}

fn levels(raw: &[(&str, &str)]) -> Vec<Level> {
    raw.iter().map(|(p, s)| lvl(p, s)).collect()
}

fn diff(
    first: u64,
    last: u64,
    prev: u64,
    bids: &[(&str, &str)],
    asks: &[(&str, &str)],
) -> DepthDiff {
    DepthDiff {
        symbol: "BTCUSDT".into(),
        event_time: Timestamp::from_millis(0),
        first_id: first,
        final_id: last,
        prev_final_id: prev,
        bids: levels(bids),
        asks: levels(asks),
    }
}

fn snapshot(id: u64, bids: &[(&str, &str)], asks: &[(&str, &str)]) -> Snapshot {
    Snapshot {
        last_update_id: id,
        bids: levels(bids),
        asks: levels(asks),
    }
}

fn secs(s: i64) -> Timestamp {
    Timestamp::from_micros(s * 1_000_000)
}

const BIDS: &[(&str, &str)] = &[("100.0", "5"), ("99.5", "4"), ("99.0", "3")];
const ASKS: &[(&str, &str)] = &[("100.1", "5"), ("100.5", "4"), ("101.0", "3")];

fn live_sync() -> BookSync {
    let mut sync = BookSync::new(1000);
    assert_eq!(
        sync.on_diff(diff(1, 5, 0, &[], &[]), secs(0)),
        Some(Action::FetchSnapshot)
    );
    assert_eq!(sync.on_snapshot(snapshot(3, BIDS, ASKS), secs(0)), None);
    assert_eq!(sync.status(), BookStatus::Live);
    sync
}

fn drifted(first: u64, last: u64, prev: u64) -> DepthDiff {
    diff(
        first,
        last,
        prev,
        &[("100.5", "1")],
        &[("100.1", "0"), ("100.5", "0")],
    )
}

#[test]
fn parses_a_usdm_depth_update_frame() {
    let text = r#"{"stream":"btcusdt@depth@100ms","data":{"e":"depthUpdate","E":1700000000123,"T":1700000000120,"s":"BTCUSDT","U":100,"u":105,"pu":99,"b":[["99.9","2"],["99.8","0"]],"a":[["100.0","3.5"]]}}"#;

    let parsed = parse_frame(text).unwrap().unwrap();

    assert_eq!(parsed.symbol, "BTCUSDT");
    assert_eq!(parsed.event_time, Timestamp::from_millis(1_700_000_000_123));
    assert_eq!(
        (parsed.first_id, parsed.final_id, parsed.prev_final_id),
        (100, 105, 99)
    );
    assert_eq!(parsed.bids, vec![lvl("99.9", "2"), lvl("99.8", "0")]);
    assert_eq!(parsed.asks, vec![lvl("100.0", "3.5")]);
    assert_eq!(parse_frame(r#"{"result":null,"id":1}"#).unwrap(), None);
    assert!(parse_frame("nope").is_err());
}

#[test]
fn parses_a_rest_depth_snapshot() {
    let text = r#"{"lastUpdateId":1027024,"E":1589436922972,"T":1589436922959,"bids":[["4.00000000","431.00000000"]],"asks":[["4.00000200","12.00000000"]]}"#;

    let parsed = parse_snapshot(text).unwrap();

    assert_eq!(parsed.last_update_id, 1_027_024);
    assert_eq!(parsed.bids, vec![lvl("4.00000000", "431.00000000")]);
    assert_eq!(parsed.asks, vec![lvl("4.00000200", "12.00000000")]);
}

#[test]
fn aligns_on_the_diff_that_straddles_the_snapshot_and_goes_live() {
    let mut sync = BookSync::new(1000);

    assert_eq!(
        sync.on_diff(diff(1, 4, 0, &[("98.0", "9")], &[]), secs(0)),
        Some(Action::FetchSnapshot)
    );
    assert_eq!(sync.status(), BookStatus::Resyncing);
    assert_eq!(
        sync.on_diff(diff(5, 8, 4, &[("99.0", "7")], &[]), secs(0)),
        None
    );
    assert_eq!(sync.on_snapshot(snapshot(6, BIDS, ASKS), secs(0)), None);

    assert_eq!(sync.status(), BookStatus::Live);
    let book = sync.book().unwrap();
    assert_eq!(
        book.levels(BookSide::Bid),
        vec![lvl("100.0", "5"), lvl("99.5", "4"), lvl("99.0", "7")]
    );
    assert_eq!(book.known_range(), Some((d("99.0"), d("101.0"))));
}

#[test]
fn drops_buffered_diffs_older_than_the_snapshot_but_keeps_an_equal_one() {
    let mut sync = BookSync::new(1000);
    sync.on_diff(diff(1, 4, 0, &[("99.0", "9")], &[]), secs(0));
    sync.on_diff(diff(5, 5, 4, &[("99.5", "8")], &[]), secs(0));

    assert_eq!(sync.on_snapshot(snapshot(5, BIDS, ASKS), secs(0)), None);

    let book = sync.book().unwrap();
    assert_eq!(
        book.levels(BookSide::Bid),
        vec![lvl("100.0", "5"), lvl("99.5", "8"), lvl("99.0", "3")]
    );
}

#[test]
fn a_snapshot_older_than_every_buffered_diff_requests_another() {
    let mut sync = BookSync::new(1000);
    sync.on_diff(diff(10, 12, 9, &[], &[]), secs(0));
    sync.on_diff(diff(13, 15, 12, &[], &[]), secs(0));

    assert_eq!(
        sync.on_snapshot(snapshot(5, BIDS, ASKS), secs(0)),
        Some(Action::FetchSnapshot)
    );
    assert_eq!(sync.status(), BookStatus::Resyncing);
    assert!(sync.book().is_none());
}

#[test]
fn a_snapshot_newer_than_every_buffered_diff_waits_for_the_next_diff() {
    let mut sync = BookSync::new(1000);
    sync.on_diff(diff(1, 5, 0, &[], &[]), secs(0));

    assert_eq!(sync.on_snapshot(snapshot(20, BIDS, ASKS), secs(0)), None);
    assert_eq!(sync.status(), BookStatus::Resyncing);
    assert_eq!(
        sync.on_diff(diff(18, 22, 17, &[("99.5", "1")], &[]), secs(0)),
        None
    );
    assert_eq!(sync.status(), BookStatus::Live);
    assert_eq!(
        sync.book().unwrap().levels(BookSide::Bid)[1],
        lvl("99.5", "1")
    );

    let mut late = BookSync::new(1000);
    late.on_diff(diff(1, 5, 0, &[], &[]), secs(0));
    late.on_snapshot(snapshot(20, BIDS, ASKS), secs(0));
    assert_eq!(
        late.on_diff(diff(25, 30, 22, &[], &[]), secs(0)),
        Some(Action::FetchSnapshot)
    );
    assert_eq!(late.status(), BookStatus::Resyncing);
}

#[test]
fn a_pu_break_while_live_resyncs_and_hides_the_book() {
    let mut sync = live_sync();

    assert_eq!(sync.on_diff(diff(6, 7, 5, &[], &[]), secs(1)), None);
    assert_eq!(
        sync.on_diff(diff(9, 10, 8, &[], &[]), secs(1)),
        Some(Action::FetchSnapshot)
    );

    assert_eq!(sync.status(), BookStatus::Resyncing);
    assert!(sync.book().is_none());
}

#[test]
fn buffer_overflow_restarts_the_resync() {
    let mut sync = BookSync::new(3);

    assert_eq!(
        sync.on_diff(diff(1, 1, 0, &[], &[]), secs(0)),
        Some(Action::FetchSnapshot)
    );
    assert_eq!(sync.on_diff(diff(2, 2, 1, &[], &[]), secs(0)), None);
    assert_eq!(sync.on_diff(diff(3, 3, 2, &[], &[]), secs(0)), None);
    assert_eq!(
        sync.on_diff(diff(4, 4, 3, &[], &[]), secs(0)),
        Some(Action::FetchSnapshot)
    );
    assert_eq!(sync.status(), BookStatus::Resyncing);
}

#[test]
fn drift_past_a_quarter_of_the_range_requests_a_recentre_no_more_than_every_10s() {
    let mut sync = live_sync();

    assert_eq!(
        sync.on_diff(diff(6, 7, 5, &[("100.4", "1")], &[]), secs(5)),
        None
    );
    assert_eq!(sync.on_diff(drifted(8, 9, 7), secs(5)), None);
    assert_eq!(
        sync.on_diff(drifted(10, 11, 9), secs(11)),
        Some(Action::FetchSnapshot)
    );
    assert_eq!(sync.on_diff(drifted(12, 13, 11), secs(12)), None);
    assert_eq!(sync.status(), BookStatus::Live);
}

#[test]
fn mid_outside_the_known_range_requests_a_snapshot() {
    let mut sync = live_sync();
    let escaped = diff(
        6,
        7,
        5,
        &[("101.4", "1"), ("100.0", "0"), ("99.5", "0"), ("99.0", "0")],
        &[
            ("101.6", "1"),
            ("100.1", "0"),
            ("100.5", "0"),
            ("101.0", "0"),
        ],
    );

    assert_eq!(sync.on_diff(escaped, secs(3)), None);
    assert_eq!(
        sync.on_diff(diff(8, 9, 7, &[], &[]), secs(11)),
        Some(Action::FetchSnapshot)
    );
}

#[test]
fn a_recentre_snapshot_swaps_in_without_leaving_live() {
    let mut sync = live_sync();
    assert_eq!(
        sync.on_diff(drifted(6, 7, 5), secs(11)),
        Some(Action::FetchSnapshot)
    );
    assert_eq!(
        sync.on_diff(diff(8, 9, 7, &[("101.2", "6")], &[]), secs(11)),
        None
    );

    let new_bids = &[("101.1", "2"), ("100.0", "1")];
    let new_asks = &[("101.6", "2"), ("102.0", "1")];
    assert_eq!(
        sync.on_snapshot(snapshot(8, new_bids, new_asks), secs(11)),
        None
    );

    assert_eq!(sync.status(), BookStatus::Live);
    let book = sync.book().unwrap();
    assert_eq!(book.known_range(), Some((d("100.0"), d("102.0"))));
    assert_eq!(book.best_bid(), Some(lvl("101.2", "6")));
}

#[test]
fn a_misaligned_recentre_snapshot_is_discarded() {
    let mut sync = live_sync();
    assert_eq!(
        sync.on_diff(drifted(6, 7, 5), secs(11)),
        Some(Action::FetchSnapshot)
    );

    assert_eq!(
        sync.on_snapshot(snapshot(2, &[("50.0", "1")], &[("51.0", "1")]), secs(11)),
        None
    );

    assert_eq!(sync.status(), BookStatus::Live);
    assert_eq!(
        sync.book().unwrap().known_range(),
        Some((d("99.0"), d("101.0")))
    );
}

#[test]
fn disconnect_drops_the_book() {
    let mut sync = live_sync();

    sync.on_disconnect();

    assert_eq!(sync.status(), BookStatus::Disconnected);
    assert!(sync.book().is_none());
    assert_eq!(
        sync.on_diff(diff(50, 51, 49, &[], &[]), secs(20)),
        Some(Action::FetchSnapshot)
    );
    assert_eq!(sync.status(), BookStatus::Resyncing);
}

#[test]
fn a_failed_snapshot_while_resyncing_asks_again_and_stays_resyncing() {
    let mut sync = BookSync::new(1000);
    sync.on_diff(diff(1, 5, 0, &[], &[]), secs(0));

    assert_eq!(sync.on_snapshot_failed(), Some(Action::FetchSnapshot));
    assert_eq!(sync.status(), BookStatus::Resyncing);
}

#[test]
fn a_failed_recentre_snapshot_abandons_the_recentre_and_stays_live() {
    let mut sync = live_sync();
    assert_eq!(
        sync.on_diff(drifted(6, 7, 5), secs(11)),
        Some(Action::FetchSnapshot)
    );

    assert_eq!(sync.on_snapshot_failed(), None);

    assert_eq!(sync.status(), BookStatus::Live);
    assert_eq!(
        sync.on_snapshot(snapshot(7, &[("50.0", "1")], &[("51.0", "1")]), secs(12)),
        None
    );
    assert_eq!(
        sync.book().unwrap().known_range(),
        Some((d("99.0"), d("101.0")))
    );
}

#[test]
fn a_recentre_waiting_longer_than_max_buffer_diffs_is_abandoned() {
    let mut sync = BookSync::new(3);
    assert_eq!(
        sync.on_diff(diff(1, 5, 0, &[], &[]), secs(0)),
        Some(Action::FetchSnapshot)
    );
    assert_eq!(sync.on_snapshot(snapshot(3, BIDS, ASKS), secs(0)), None);
    assert_eq!(
        sync.on_diff(drifted(6, 7, 5), secs(11)),
        Some(Action::FetchSnapshot)
    );
    for id in 8..=11 {
        assert_eq!(sync.on_diff(diff(id, id, id - 1, &[], &[]), secs(12)), None);
    }

    assert_eq!(
        sync.on_snapshot(snapshot(10, &[("50.0", "1")], &[("51.0", "1")]), secs(12)),
        None
    );

    assert_eq!(sync.status(), BookStatus::Live);
    assert_eq!(
        sync.book().unwrap().known_range(),
        Some((d("99.0"), d("101.0")))
    );
}
