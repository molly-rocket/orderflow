use std::path::PathBuf;

use crate::Fixture;

pub fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

pub fn load(name: &str) -> Fixture {
    let path = dir().join(name);
    Fixture::load(&path).unwrap_or_else(|e| panic!("fixture {}: {e}", path.display()))
}

pub mod btc_perp {
    pub const FILE: &str = "binance-perp-btcusdt-aggtrade.jsonl";
    pub const TRADES: u64 = 6_400;
    pub const BUY_VOLUME: &str = "1074.077";
    pub const SELL_VOLUME: &str = "293.208";
    pub const FIRST_TRADE_MS: i64 = 1_794_179_725_753;
    pub const LAST_TRADE_MS: i64 = 1_794_179_745_842;
    pub const TICK: &str = "0.1";
}

pub fn btc_perp() -> Fixture {
    load(btc_perp::FILE)
}
