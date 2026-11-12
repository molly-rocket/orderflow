use orderflow_e2e::Stack;
use serde_json::Value;
use venue_sim::binance::AggTrades;
use venue_sim::{Fault, Fixture, Script};

const MINUTE_MS: i64 = 60_000;
const MIDNIGHT_MS: i64 = 20_765 * 86_400_000;

fn us(ms: i64) -> i64 {
    ms * 1_000
}

fn across_midnight() -> Fixture {
    AggTrades::new("BTCUSDT")
        .buy(MIDNIGHT_MS - 2 * MINUTE_MS, "100", "1")
        .sell(MIDNIGHT_MS - MINUTE_MS, "100", "0.5")
        .buy(MIDNIGHT_MS, "100", "2")
        .sell(MIDNIGHT_MS + MINUTE_MS, "100", "1")
        .build()
}

fn cvd(body: &Value) -> Vec<(String, bool)> {
    body["bars"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            (
                b["cvd"].as_str().unwrap().to_owned(),
                b["cvd_complete"].as_bool().unwrap(),
            )
        })
        .collect()
}

fn points(expected: &[(&str, bool)]) -> Vec<(String, bool)> {
    expected
        .iter()
        .map(|(v, c)| ((*v).to_owned(), *c))
        .collect()
}

#[tokio::test]
async fn cvd_accumulates_through_the_trading_day_and_resets_at_00_00_utc() {
    let stack = Stack::start(Script::new(across_midnight())).await;
    let (from, to) = (
        us(MIDNIGHT_MS - 2 * MINUTE_MS),
        us(MIDNIGHT_MS + 2 * MINUTE_MS),
    );
    stack.wait_for_trades(from, to, 4).await;

    let all = stack.footprint(from, to, "&timeframe_secs=60").await;
    assert_eq!(
        cvd(&all),
        points(&[("1", true), ("0.5", true), ("2", true), ("1", true)])
    );

    let mid_day = stack
        .footprint(us(MIDNIGHT_MS + MINUTE_MS), to, "&timeframe_secs=60")
        .await;
    assert_eq!(cvd(&mid_day), points(&[("1", true)]));

    let late_evening = stack
        .footprint(
            us(MIDNIGHT_MS - MINUTE_MS),
            us(MIDNIGHT_MS),
            "&timeframe_secs=60",
        )
        .await;
    assert_eq!(cvd(&late_evening), points(&[("0.5", true)]));
    stack.stop().await;
}

#[tokio::test]
async fn a_gap_makes_cvd_incomplete_for_the_rest_of_its_trading_day_only() {
    let script = Script::new(across_midnight()).with_fault(Fault::Disconnect {
        after_frames: 2,
        lost_frames: 0,
    });
    let stack = Stack::start(script).await;
    let (from, to) = (
        us(MIDNIGHT_MS - 2 * MINUTE_MS),
        us(MIDNIGHT_MS + 2 * MINUTE_MS),
    );
    stack.wait_for_trades(from, to, 4).await;

    let body = stack.footprint(from, to, "&timeframe_secs=60").await;

    assert_eq!(
        body["gaps"][0]["start_us"].as_i64(),
        Some(us(MIDNIGHT_MS - MINUTE_MS))
    );
    assert_eq!(
        cvd(&body),
        points(&[("1", true), ("0.5", false), ("2", true), ("1", true)])
    );
    stack.stop().await;
}
