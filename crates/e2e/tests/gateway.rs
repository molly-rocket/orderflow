use std::time::{SystemTime, UNIX_EPOCH};

use orderflow_domain::Decimal;
use orderflow_e2e::{MARKET, Stack, captures};
use serde_json::{Value, json};
use venue_sim::binance::AggTrades;
use venue_sim::{Fault, Fixture, Script};

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

fn dec(v: &Value) -> Decimal {
    v.as_str()
        .unwrap_or_else(|| panic!("{v} is not a decimal string"))
        .parse()
        .unwrap()
}

fn us(ms: i64) -> i64 {
    ms * 1_000
}

#[tokio::test]
async fn the_footprint_of_a_real_capture_matches_its_independently_summed_totals() {
    use captures::btc_perp as cap;
    let stack = Stack::start(Script::new(captures::btc_perp().expect("btc perp capture"))).await;
    let from = us(cap::FIRST_TRADE_MS / MINUTE_MS * MINUTE_MS);
    let to = us(cap::LAST_TRADE_MS + 1);

    stack.wait_for_trades(from, to, cap::TRADES).await;
    let body = stack
        .footprint(from, to, "&timeframe_secs=60&grouping=1")
        .await;

    let bars = body["bars"].as_array().unwrap();
    let total = |key: &str| bars.iter().map(|b| dec(&b[key])).sum::<Decimal>();
    assert_eq!(total("buy"), cap::BUY_VOLUME.parse::<Decimal>().unwrap());
    assert_eq!(total("sell"), cap::SELL_VOLUME.parse::<Decimal>().unwrap());
    assert_eq!(
        bars.iter()
            .map(|b| b["trades"].as_u64().unwrap())
            .sum::<u64>(),
        cap::TRADES
    );
    for bar in bars {
        let rows = bar["rows"].as_array().unwrap();
        let rows_buy: Decimal = rows.iter().map(|r| dec(&r["buy"])).sum();
        let rows_sell: Decimal = rows.iter().map(|r| dec(&r["sell"])).sum();
        assert_eq!(rows_buy, dec(&bar["buy"]));
        assert_eq!(rows_sell, dec(&bar["sell"]));
        assert_eq!(dec(&bar["delta"]), rows_buy - rows_sell);
        assert_eq!(bar["incomplete"], false);
        let prices: Vec<Decimal> = rows.iter().map(|r| dec(&r["price"])).collect();
        assert!(prices.windows(2).all(|w| w[0] < w[1]));
        assert!(prices.iter().all(|p| (p * Decimal::TEN).fract().is_zero()));
    }
    assert_eq!(body["gaps"], json!([]));

    let (status, markets) = stack.get("/api/v1/markets").await;
    assert_eq!(status, 200);
    assert_eq!(
        markets,
        json!([{
            "id": MARKET,
            "venue": "binance",
            "market_type": "perp",
            "native_name": "BTCUSDT",
            "tick_size": "0.1",
            "recording_horizon_us": us(cap::FIRST_TRADE_MS),
            "burst_grouped": true,
        }])
    );
    stack.stop().await;
}

#[tokio::test]
async fn imbalances_and_stacks_are_flagged_at_the_requested_price_grouping() {
    let t0 = recent_minute(30);
    let fixture = AggTrades::new("BTCUSDT")
        .sell(t0, "100.0", "1")
        .buy(t0 + 1, "100.1", "3")
        .sell(t0 + 2, "100.1", "1")
        .buy(t0 + 3, "100.2", "3")
        .sell(t0 + 4, "100.2", "1")
        .buy(t0 + 5, "100.3", "3")
        .build();
    let stack = Stack::start(Script::new(fixture)).await;
    let (from, to) = (us(t0), us(t0 + MINUTE_MS));
    stack.wait_for_trades(from, to, 6).await;

    let fine = stack.footprint(from, to, "&grouping=1").await;
    assert_eq!(
        fine["bars"][0]["rows"],
        json!([
            {"price": "100",   "buy": "0", "sell": "1", "imbalance": null},
            {"price": "100.1", "buy": "3", "sell": "1", "imbalance": "buy"},
            {"price": "100.2", "buy": "3", "sell": "1", "imbalance": "buy"},
            {"price": "100.3", "buy": "3", "sell": "0", "imbalance": "buy"},
        ])
    );
    assert_eq!(
        fine["bars"][0]["stacks"],
        json!([{"side": "buy", "low": "100.1", "high": "100.3"}])
    );

    let coarse = stack.footprint(from, to, "&grouping=2").await;
    assert_eq!(
        coarse["bars"][0]["rows"],
        json!([
            {"price": "100",   "buy": "3", "sell": "2", "imbalance": null},
            {"price": "100.2", "buy": "6", "sell": "1", "imbalance": "buy"},
        ])
    );
    assert_eq!(coarse["bars"][0]["stacks"], json!([]));
    assert_eq!(
        coarse["grouping"],
        json!({"tick": "0.1", "multiple": 2, "step": "0.2"})
    );

    let strict = stack.footprint(from, to, "&imbalance_ratio=4").await;
    assert!(
        strict["bars"][0]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["imbalance"].is_null())
    );
    stack.stop().await;
}

#[tokio::test]
async fn footprint_bars_report_open_high_low_close() {
    let t0 = recent_minute(40);
    let fixture = AggTrades::new("BTCUSDT")
        .buy(t0, "100.3", "1")
        .sell(t0 + 1, "101.7", "1")
        .buy(t0 + 2, "99.9", "1")
        .sell(t0 + 3, "100.8", "1")
        .build();
    let stack = Stack::start(Script::new(fixture)).await;
    let (from, to) = (us(t0), us(t0 + MINUTE_MS));
    stack.wait_for_trades(from, to, 4).await;

    let bar = &stack.footprint(from, to, "&grouping=1").await["bars"][0];
    assert_eq!(bar["open"], json!("100.3"));
    assert_eq!(bar["high"], json!("101.7"));
    assert_eq!(bar["low"], json!("99.9"));
    assert_eq!(bar["close"], json!("100.8"));
    stack.stop().await;
}

#[tokio::test]
async fn a_live_session_catches_up_before_sending_its_first_bar() {
    let t0 = recent_minute(1);
    let mut fixture = AggTrades::new("BTCUSDT");
    for i in 0..4_000 {
        fixture = fixture.buy(t0 + i, &format!("100.{}", i % 10), "1");
    }
    let stack = Stack::start(Script::new(fixture.build())).await;
    let (from, to) = (us(t0), us(t0 + MINUTE_MS));
    stack.wait_for_trades(from, to, 4_000).await;

    let mut live = stack.live(&format!("&grouping=1&since_us={from}")).await;
    let messages = live
        .read_until(|seen| seen.iter().any(|m| m["type"] == json!("bar")))
        .await;

    let first = messages.iter().find(|m| m["type"] == json!("bar")).unwrap();
    assert_eq!(first["bar"]["trades"], json!(4_000));
    assert_eq!(first["bar"]["close"], json!("100.9"));
    stack.stop().await;
}

#[tokio::test]
async fn a_disconnect_is_a_gap_and_bars_overlapping_it_are_incomplete() {
    let t0 = recent_minute(30);
    let fixture = AggTrades::new("BTCUSDT")
        .buy(t0, "100", "1")
        .buy(t0 + 1_000, "100", "1")
        .buy(t0 + 2_000, "100", "1")
        .buy(t0 + 70_000, "100", "1") // lost while disconnected
        .buy(t0 + 130_000, "100", "1")
        .buy(t0 + 131_000, "100", "1")
        .buy(t0 + 200_000, "100", "1")
        .build();
    let script = Script::new(fixture).with_fault(Fault::Disconnect {
        after_frames: 3,
        lost_frames: 1,
    });
    let stack = Stack::start(script).await;
    let (from, to) = (us(t0), us(t0 + 4 * MINUTE_MS));
    stack.wait_for_trades(from, to, 6).await;

    let expected_gap = json!({
        "start_us": us(t0 + 2_000),
        "end_us": us(t0 + 130_000),
        "reason": "disconnect",
    });
    let (status, gaps) = stack
        .get(&format!(
            "/api/v1/gaps?market={MARKET}&from_us={from}&to_us={to}"
        ))
        .await;
    assert_eq!(status, 200);
    assert_eq!(gaps, json!([expected_gap]));

    let body = stack.footprint(from, to, "").await;
    let bars: Vec<(i64, bool)> = body["bars"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            (
                b["start_us"].as_i64().unwrap(),
                b["incomplete"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        bars,
        vec![
            (us(t0), true),
            (us(t0 + 2 * MINUTE_MS), true),
            (us(t0 + 3 * MINUTE_MS), false),
        ]
    );
    assert_eq!(body["gaps"], json!([expected_gap]));
    stack.stop().await;
}

#[tokio::test]
async fn live_bars_equal_history_bars_and_live_gaps_are_pushed() {
    let t0 = recent_minute(20);
    let fixture = AggTrades::new("BTCUSDT")
        .buy(t0, "100.0", "1.5")
        .sell(t0 + 10, "100.1", "0.5")
        .buy(t0 + 20, "99.9", "2")
        .sell(t0 + MINUTE_MS, "101", "1")
        .buy(t0 + MINUTE_MS + 1, "101.1", "4")
        .skip_ids(1)
        .sell(t0 + 2 * MINUTE_MS, "102", "3")
        .build();
    let stack = Stack::start(Script::new(fixture)).await;
    let mut live = stack
        .live(&format!(
            "&timeframe_secs=60&grouping=1&since_us={}",
            us(t0)
        ))
        .await;

    let messages = live
        .read_until(|m| {
            m.iter()
                .filter(|x| x["type"] == "bar" && x["closed"] == true)
                .count()
                == 2
                && m.iter().any(|x| {
                    x["type"] == "bar"
                        && x["bar"]["start_us"] == us(t0 + 2 * MINUTE_MS)
                        && x["bar"]["trades"] == 1
                })
        })
        .await;
    stack
        .wait_for_trades(us(t0), us(t0 + 3 * MINUTE_MS), 6)
        .await;
    let history = stack
        .footprint(
            us(t0),
            us(t0 + 3 * MINUTE_MS),
            "&timeframe_secs=60&grouping=1",
        )
        .await;

    let live_bars: Vec<&Value> = messages
        .iter()
        .filter(|m| m["type"] == "bar" && m["closed"] == true)
        .map(|m| &m["bar"])
        .chain(
            messages
                .iter()
                .rev()
                .find(|m| m["type"] == "bar" && m["bar"]["start_us"] == us(t0 + 2 * MINUTE_MS))
                .map(|m| &m["bar"]),
        )
        .collect();
    let history_bars: Vec<&Value> = history["bars"].as_array().unwrap().iter().collect();
    assert_eq!(live_bars, history_bars);

    let gap = json!({
        "start_us": us(t0 + MINUTE_MS + 1),
        "end_us": us(t0 + 2 * MINUTE_MS),
        "reason": "sequence_break",
    });
    assert!(
        messages
            .iter()
            .any(|m| m["type"] == "gap" && m["gap"] == gap),
        "{messages:#?}"
    );
    assert_eq!(history["gaps"], json!([gap]));
    stack.stop().await;
}

#[tokio::test]
async fn invalid_requests_get_structured_errors() {
    let stack = Stack::start(Script::new(Fixture::default())).await;
    let range = "&from_us=1000000&to_us=2000000";
    let cases = [
        (
            format!("/api/v1/footprint?market={MARKET}{range}&grouping=0"),
            400,
        ),
        (
            format!("/api/v1/footprint?market={MARKET}{range}&timeframe_secs=30"),
            400,
        ),
        (
            format!("/api/v1/footprint?market={MARKET}{range}&timeframe_secs=7"),
            400,
        ),
        (
            format!("/api/v1/footprint?market={MARKET}{range}&imbalance_ratio=0.5"),
            400,
        ),
        (
            format!("/api/v1/footprint?market={MARKET}&from_us=5&to_us=5"),
            400,
        ),
        (
            format!("/api/v1/footprint?market={MARKET}&from_us=0&to_us=1000000000000000"),
            400,
        ),
        (
            format!("/api/v1/footprint?market=binance:perp:ETHUSDT{range}"),
            404,
        ),
        (format!("/api/v1/footprint?market=nonsense{range}"), 400),
    ];
    for (path, expected) in cases {
        let (status, body) = stack.get(&path).await;
        assert_eq!(status, expected, "{path}: {body}");
        assert!(
            body["error"].as_str().is_some_and(|e| !e.is_empty()),
            "{path}: {body}"
        );
    }
    stack.stop().await;
}

#[tokio::test]
async fn a_storage_outage_answers_a_retryable_503_and_recovers() {
    let stack = Stack::start(Script::new(Fixture::default())).await;
    let range = "&from_us=60000000&to_us=120000000";
    let paths = [
        "/api/v1/markets".to_owned(),
        format!("/api/v1/footprint?market={MARKET}{range}"),
        format!("/api/v1/gaps?market={MARKET}{range}"),
    ];
    for path in &paths {
        assert_eq!(stack.get(path).await.0, 200, "{path} before the outage");
    }

    stack.cut_storage();
    for path in &paths {
        let response = stack.get_response(path).await;
        assert_eq!(response.status().as_u16(), 503, "{path} during the outage");
        assert!(
            response.headers().contains_key("retry-after"),
            "{path}: 503 without Retry-After"
        );
        let body: Value = response.json().await.unwrap();
        assert_eq!(body, json!({"error": "storage unavailable"}), "{path}");
    }

    stack.restore_storage();
    for path in &paths {
        assert_eq!(stack.get(path).await.0, 200, "{path} after the outage");
    }
    stack.stop().await;
}
