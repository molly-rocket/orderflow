use orderflow_e2e::captures;
use serde_json::Value;

#[test]
fn the_binance_perp_capture_is_contiguous_btcusdt_agg_trades() {
    let fixture = captures::btc_perp();
    let ids: Vec<u64> = fixture
        .frames()
        .iter()
        .map(|frame| {
            let v: Value = serde_json::from_str(&frame.text).unwrap();
            assert_eq!(v["stream"], "btcusdt@aggTrade");
            assert_eq!(v["data"]["s"], "BTCUSDT");
            v["data"]["a"].as_u64().unwrap()
        })
        .collect();

    assert_eq!(ids.len() as u64, captures::btc_perp::TRADES);
    assert!(ids.windows(2).all(|w| w[1] == w[0] + 1));
}
