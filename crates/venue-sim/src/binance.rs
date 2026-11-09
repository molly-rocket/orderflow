use serde_json::json;

use crate::{Fixture, Frame};

#[derive(Debug, Clone)]
pub struct AggTrades {
    symbol: String,
    next_agg_id: u64,
    next_trade_id: u64,
    frames: Vec<Frame>,
}

impl AggTrades {
    pub fn new(symbol: &str) -> Self {
        Self {
            symbol: symbol.to_owned(),
            next_agg_id: 1_000,
            next_trade_id: 50_000,
            frames: Vec::new(),
        }
    }

    #[must_use]
    pub fn buy(self, time_ms: i64, price: &str, qty: &str) -> Self {
        self.push(time_ms, price, qty, false, 1)
    }

    #[must_use]
    pub fn sell(self, time_ms: i64, price: &str, qty: &str) -> Self {
        self.push(time_ms, price, qty, true, 1)
    }

    #[must_use]
    pub fn skip_ids(mut self, n: u64) -> Self {
        self.next_agg_id += n;
        self.next_trade_id += n;
        self
    }

    pub fn build(self) -> Fixture {
        Fixture::new(self.frames)
    }

    fn push(
        mut self,
        time_ms: i64,
        price: &str,
        qty: &str,
        buyer_is_maker: bool,
        fills: u64,
    ) -> Self {
        let first = self.next_trade_id;
        let last = first + fills - 1;
        let stream = format!("{}@aggTrade", self.symbol.to_lowercase());
        let text = json!({
            "stream": stream,
            "data": {
                "e": "aggTrade",
                "E": time_ms + 5,
                "s": self.symbol,
                "a": self.next_agg_id,
                "p": price,
                "q": qty,
                "f": first,
                "l": last,
                "T": time_ms,
                "m": buyer_is_maker,
            }
        })
        .to_string();
        self.frames.push(Frame {
            recv_us: (time_ms + 5) * 1_000,
            text,
        });
        self.next_agg_id += 1;
        self.next_trade_id = last + 1;
        self
    }
}
