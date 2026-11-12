use std::time::{Duration, SystemTime, UNIX_EPOCH};

use orderflow_domain::Decimal;
use orderflow_e2e::{MARKET, Stack, captures};
use serde_json::{Value, json};
use venue_sim::binance::AggTrades;
use venue_sim::{Fixture, Pacing, Script};

const MINUTE_MS: i64 = 60_000;

fn recent_minute(minutes_ago: i64) -> i64 {
    let now_ms = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    (now_ms / MINUTE_MS - minutes_ago) * MINUTE_MS
}

fn us(ms: i64) -> i64 {
    ms * 1_000
}

fn dec(v: &Value) -> Decimal {
    v.as_str()
        .unwrap_or_else(|| panic!("{v} is not a decimal string"))
        .parse()
        .unwrap()
}

fn three_bubbles(t0: i64) -> Fixture {
    AggTrades::new("BTCUSDT")
        .buy(t0, "100", "1")
        .buy(t0 + 40, "101", "2")
        .buy(t0 + 99, "102", "1")
        .sell(t0 + 120, "99", "0.5")
        .buy(t0 + 300, "100", "2")
        .build()
}

fn first_ids(body: &Value) -> Vec<u64> {
    body["bubbles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["first_trade_id"].as_u64().unwrap())
        .collect()
}

fn large_ids(body: &Value) -> Vec<u64> {
    body["bubbles"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["large"] == true)
        .map(|b| b["first_trade_id"].as_u64().unwrap())
        .collect()
}

#[tokio::test]
async fn bubbles_from_a_hand_written_fixture_match_hand_worked_values() {
    let t0 = recent_minute(30);
    let stack = Stack::start(Script::new(three_bubbles(t0))).await;
    let body = stack
        .wait_for_bubble_trades(us(t0), us(t0 + MINUTE_MS), 5)
        .await;
    let first = body["bubbles"][0]["first_trade_id"].as_u64().unwrap();

    assert_eq!(
        body["bubbles"],
        json!([
            {
                "start_us": us(t0), "end_us": us(t0 + 99), "side": "buy",
                "vwap": "101", "size": "4", "notional": "404", "trades": 3,
                "low": "100", "high": "102", "first_trade_id": first, "large": true
            },
            {
                "start_us": us(t0 + 120), "end_us": us(t0 + 120), "side": "sell",
                "vwap": "99", "size": "0.5", "notional": "49.5", "trades": 1,
                "low": "99", "high": "99", "first_trade_id": first + 3, "large": false
            },
            {
                "start_us": us(t0 + 300), "end_us": us(t0 + 300), "side": "buy",
                "vwap": "100", "size": "2", "notional": "200", "trades": 1,
                "low": "100", "high": "100", "first_trade_id": first + 4, "large": false
            }
        ])
    );
    assert_eq!(body["burst_grouped"], true);
    assert_eq!(body["truncated"], false);
    assert_eq!(body["large"]["percentile"], "99");
    assert_eq!(body["large"]["threshold"], "404");

    let range = (us(t0), us(t0 + MINUTE_MS));
    let median = stack.bubbles(range.0, range.1, "&percentile=50").await;
    assert_eq!(median["large"]["threshold"], "200");
    assert_eq!(large_ids(&median), vec![first, first + 4]);

    let floored = stack
        .bubbles(range.0, range.1, "&percentile=50&usd_floor=300")
        .await;
    assert_eq!(large_ids(&floored), vec![first]);

    let big = stack.bubbles(range.0, range.1, "&min_notional=100").await;
    assert_eq!(first_ids(&big), vec![first, first + 4]);

    let sells = stack.bubbles(range.0, range.1, "&side=sell").await;
    assert_eq!(first_ids(&sells), vec![first + 3]);

    stack.stop().await;
}

fn assert_capture_totals(body: &Value) {
    use captures::btc_perp as cap;
    let bubbles = body["bubbles"].as_array().unwrap();
    let side_size = |side: &str| -> Decimal {
        bubbles
            .iter()
            .filter(|b| b["side"] == side)
            .map(|b| dec(&b["size"]))
            .sum()
    };
    assert_eq!(
        side_size("buy"),
        cap::BUY_VOLUME.parse::<Decimal>().unwrap()
    );
    assert_eq!(
        side_size("sell"),
        cap::SELL_VOLUME.parse::<Decimal>().unwrap()
    );
    for bubble in bubbles {
        let span = bubble["end_us"].as_i64().unwrap() - bubble["start_us"].as_i64().unwrap();
        assert!((0..100_000).contains(&span), "{bubble}");
        assert!(
            dec(&bubble["low"]) <= dec(&bubble["vwap"])
                && dec(&bubble["vwap"]) <= dec(&bubble["high"]),
            "{bubble}"
        );
    }
}

fn capture_range() -> (i64, i64) {
    use captures::btc_perp as cap;
    (us(cap::FIRST_TRADE_MS), us(cap::LAST_TRADE_MS + 1))
}

#[tokio::test]
async fn the_bubbles_of_a_real_capture_account_for_every_trade() {
    let capture = captures::btc_perp().expect("btc perp capture");
    let stack = Stack::start(Script::new(capture)).await;
    let (from, to) = capture_range();

    let body = stack
        .wait_for_bubble_trades(from, to, captures::btc_perp::TRADES)
        .await;

    assert_capture_totals(&body);
    stack.stop().await;
}

#[tokio::test]
async fn an_aggregator_restart_mid_replay_loses_and_duplicates_no_bubble() {
    let capture = captures::btc_perp().expect("btc perp capture");
    let script = Script::new(capture).with_pacing(Pacing::Realtime { speed: 4.0 });
    let mut stack = Stack::start(script).await;
    let (from, to) = capture_range();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let body = stack.bubbles(from, to, "").await;
        if body["bubbles"].as_array().unwrap().len() > 100 {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "no Bubbles yet");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    stack.restart_aggregator().await;

    let body = stack
        .wait_for_bubble_trades(from, to, captures::btc_perp::TRADES)
        .await;
    assert_capture_totals(&body);
    stack.stop().await;
}

#[tokio::test]
async fn live_bubbles_equal_history_bubbles() {
    let t0 = recent_minute(20);
    let stack = Stack::start(Script::new(three_bubbles(t0))).await;
    let history = stack
        .wait_for_bubble_trades(us(t0), us(t0 + MINUTE_MS), 5)
        .await;
    let mut live = stack
        .live(&format!("&timeframe_secs=60&since_us={}", us(t0)))
        .await;

    let messages = live
        .read_until(|m| m.iter().filter(|x| x["type"] == "bubble").count() == 3)
        .await;

    let live_bubbles: Vec<&Value> = messages
        .iter()
        .filter(|m| m["type"] == "bubble")
        .map(|m| &m["bubble"])
        .collect();
    let history_bubbles: Vec<&Value> = history["bubbles"].as_array().unwrap().iter().collect();
    assert_eq!(live_bubbles, history_bubbles);
    assert_eq!(live_bubbles[0]["large"], true);
    stack.stop().await;
}

#[tokio::test]
async fn invalid_bubble_requests_get_structured_errors() {
    let stack = Stack::start(Script::new(Fixture::default())).await;
    let range = "&from_us=1000000&to_us=2000000";
    for query in [
        "&percentile=100",
        "&percentile=0",
        "&percentile=abc",
        "&usd_floor=-1",
        "&min_notional=-5",
        "&side=up",
    ] {
        let path = format!("/api/v1/bubbles?market={MARKET}{range}{query}");
        let (status, body) = stack.get(&path).await;
        assert_eq!(status, 400, "{path}: {body}");
        assert!(
            body["error"].as_str().is_some_and(|e| !e.is_empty()),
            "{body}"
        );
    }
    let too_long = format!("/api/v1/bubbles?market={MARKET}&from_us=0&to_us=345600000000");
    assert_eq!(stack.get(&too_long).await.0, 400);
    stack.stop().await;
}
