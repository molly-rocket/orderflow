use rust_decimal::Decimal;

use crate::Timestamp;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TakerSide {
    Buy,
    Sell,
}

impl TakerSide {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
        }
    }
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trade {
    pub venue_trade_id: u64,
    pub engine_time: Timestamp,
    pub price: Decimal,
    pub size: Decimal,
    pub taker_side: TakerSide,
    pub taker_order_id: Option<Box<str>>,
}

impl Trade {
    pub fn notional(&self) -> Decimal {
        self.price * self.size
    }
}
