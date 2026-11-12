use std::time::{Duration, SystemTime, UNIX_EPOCH};

use orderflow_e2e::Stack;
use serde_json::{Value, json};
use venue_sim::binance::{AggTrades, DepthDiffs, depth_snapshot};
use venue_sim::{Fault, Pacing, Script};

const BIDS: &[(&str, &str)] = &[("100.0", "5"), ("99.5", "4"), ("99.0", "3")];
const ASKS: &[(&str, &str)] = &[("100.1", "5"), ("100.5", "4"), ("101.0", "3")];

fn now_us() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros(),
    )
    .unwrap()
}

fn trades() -> Script {
    Script::new(
        AggTrades::new("BTCUSDT")
            .buy(now_us() / 1_000, "100.0", "1")
            .build(),
    )
}

fn depth() -> Script {
    Script::new(
        DepthDiffs::new("BTCUSDT")
            .diff(1, 4, 0, &[("99.0", "9")], &[])
            .diff(5, 8, 4, &[("99.5", "8")], &[("100.5", "0")])
            .diff(9, 12, 8, &[], &[("100.9", "2")])
            .build(),
    )
}

fn expected_rows() -> Value {
    json!([
        {"price": "99", "bid": "11", "ask": "0"},
        {"price": "100", "bid": "5", "ask": "7"},
    ])
}

fn is_type(message: &Value, kind: &str) -> bool {
    message["type"] == json!(kind)
}

#[tokio::test]
async fn book_socket_sends_the_ladder_at_the_session_grouping() {
    let mut stack = Stack::start(trades()).await;
    stack
        .start_book(depth(), vec![(200, depth_snapshot(6, BIDS, ASKS))])
        .await;

    let mut socket = stack.book("&grouping=10").await;
    let messages = socket
        .read_until(|seen| {
            seen.iter()
                .any(|m| is_type(m, "book") && m["book"]["rows"] == expected_rows())
        })
        .await;

    let book = &messages.last().unwrap()["book"];
    assert_eq!(book["status"], json!("live"));
    assert_eq!(book["step"], json!("1"));
    assert_eq!(book["best_bid"], json!("100"));
    assert_eq!(book["best_ask"], json!("100.1"));
    assert_eq!(book["known_low"], json!("99"));
    assert_eq!(book["known_high"], json!("101"));
    assert!(book["time_us"].as_i64().unwrap() > 0);
    stack.stop().await;
}

#[tokio::test]
async fn book_socket_reports_resyncing_without_rows_after_a_dropped_diff() {
    let mut stack = Stack::start(trades()).await;
    let script = Script::new(
        DepthDiffs::new("BTCUSDT")
            .diff(1, 4, 0, &[], &[])
            .diff(5, 8, 4, &[], &[])
            .diff(9, 12, 8, &[], &[])
            .diff(13, 16, 12, &[("200.0", "3")], &[])
            .build(),
    )
    .with_pacing(Pacing::Realtime { speed: 1.0 })
    .with_fault(Fault::Drop {
        at_frame: 2,
        count: 1,
    });
    let mut snapshots = vec![(200, depth_snapshot(6, BIDS, ASKS))];
    snapshots.extend(std::iter::repeat_n((429, "{}".to_owned()), 8));
    snapshots.push((
        200,
        depth_snapshot(14, &[("200.0", "1")], &[("200.1", "1")]),
    ));
    stack.start_book(script, snapshots).await;

    let mut socket = stack.book("&grouping=1").await;
    let messages = socket
        .read_until(|seen| {
            seen.iter()
                .any(|m| is_type(m, "book") && m["book"]["status"] == json!("live"))
                && seen
                    .iter()
                    .any(|m| is_type(m, "book") && m["book"]["status"] == json!("resyncing"))
        })
        .await;

    let resyncing = messages
        .iter()
        .find(|m| m["book"]["status"] == json!("resyncing"))
        .unwrap();
    assert_eq!(resyncing["book"]["rows"], json!([]));
    assert_eq!(resyncing["book"]["best_bid"], Value::Null);
    assert_eq!(resyncing["book"]["known_low"], Value::Null);
    stack.stop().await;
}

#[tokio::test]
async fn book_socket_reports_disconnected_when_no_state_arrives() {
    let stack = Stack::start(trades()).await;

    let mut socket = stack.book("&grouping=1").await;
    let started = tokio::time::Instant::now();
    let messages = socket
        .read_until(|seen| seen.iter().any(|m| is_type(m, "book")))
        .await;

    assert!(started.elapsed() < Duration::from_secs(4));
    let book = &messages.last().unwrap()["book"];
    assert_eq!(book["status"], json!("disconnected"));
    assert_eq!(book["rows"], json!([]));
    stack.stop().await;
}

#[tokio::test]
async fn heatmap_history_then_live_frames_when_requested() {
    let mut stack = Stack::start(trades()).await;
    stack
        .start_book(depth(), vec![(200, depth_snapshot(6, BIDS, ASKS))])
        .await;
    tokio::time::sleep(Duration::from_millis(3_200)).await;

    let since = now_us() - 10_000_000;
    let mut socket = stack
        .book(&format!("&grouping=10&heatmap_since_us={since}"))
        .await;
    let messages = socket
        .read_until(|seen| seen.iter().any(|m| is_type(m, "heatmap")))
        .await;

    let history = messages
        .iter()
        .find(|m| is_type(m, "heatmap_history"))
        .expect("history before live frames");
    let frames = history["frames"].as_array().unwrap();
    assert!(frames.len() >= 2, "{} frames", frames.len());
    for frame in frames {
        assert_eq!(frame["status"], json!("live"));
        assert_eq!(frame["step"], json!("1"));
        assert_eq!(frame["low"], json!("99"));
        assert_eq!(frame["sizes"], json!([11.0, 12.0]));
    }
    let live = messages.iter().find(|m| is_type(m, "heatmap")).unwrap();
    assert_eq!(live["frame"]["sizes"], json!([11.0, 12.0]));
    assert!(
        messages.iter().position(|m| is_type(m, "heatmap_history"))
            < messages.iter().position(|m| is_type(m, "heatmap"))
    );
    stack.stop().await;
}

#[tokio::test]
async fn heatmap_uses_the_base_grouping_when_the_session_is_finer() {
    let mut stack = Stack::start(trades()).await;
    stack
        .start_book(depth(), vec![(200, depth_snapshot(6, BIDS, ASKS))])
        .await;

    let since = now_us() - 10_000_000;
    let mut socket = stack
        .book(&format!("&grouping=1&heatmap_since_us={since}"))
        .await;
    let messages = socket
        .read_until(|seen| {
            seen.iter().any(|m| is_type(m, "heatmap"))
                && seen
                    .iter()
                    .any(|m| is_type(m, "book") && m["book"]["status"] == json!("live"))
        })
        .await;

    let frame = &messages.iter().find(|m| is_type(m, "heatmap")).unwrap()["frame"];
    assert_eq!(frame["step"], json!("1"));
    let book = &messages
        .iter()
        .find(|m| is_type(m, "book") && m["book"]["status"] == json!("live"))
        .unwrap()["book"];
    assert_eq!(book["step"], json!("0.1"));
    assert_eq!(
        book["rows"][0],
        json!({"price": "99", "bid": "3", "ask": "0"})
    );
    stack.stop().await;
}

#[tokio::test]
async fn no_heatmap_is_sent_without_heatmap_since_us() {
    let mut stack = Stack::start(trades()).await;
    stack
        .start_book(depth(), vec![(200, depth_snapshot(6, BIDS, ASKS))])
        .await;

    let mut socket = stack.book("&grouping=10").await;
    let messages = socket
        .read_until(|seen| seen.iter().filter(|m| is_type(m, "book")).count() >= 8)
        .await;

    assert!(messages.iter().all(|m| is_type(m, "book")));
    stack.stop().await;
}
