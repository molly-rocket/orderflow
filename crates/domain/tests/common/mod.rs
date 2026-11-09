#![allow(dead_code, unreachable_pub)]

use orderflow_domain::{Decimal, TakerSide, Timestamp, Trade};

pub fn d(s: &str) -> Decimal {
    s.parse()
        .unwrap_or_else(|_| panic!("bad decimal literal {s:?}"))
}

pub fn trade(time_us: i64, price: &str, size: &str, taker: TakerSide) -> Trade {
    Trade {
        venue_trade_id: u64::try_from(time_us).unwrap_or_default(),
        engine_time: Timestamp::from_micros(time_us),
        price: d(price),
        size: d(size),
        taker_side: taker,
        taker_order_id: None,
    }
}

pub fn buy(time_us: i64, price: &str, size: &str) -> Trade {
    trade(time_us, price, size, TakerSide::Buy)
}

pub fn sell(time_us: i64, price: &str, size: &str) -> Trade {
    trade(time_us, price, size, TakerSide::Sell)
}
