use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::{TakerSide, Timestamp, Trade};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BubbleGrouping {
    window_micros: i64,
}

impl BubbleGrouping {
    pub const BINANCE_PERP: Self = Self::burst_millis(100);

    pub const fn burst_millis(window_ms: u32) -> Self {
        Self {
            window_micros: window_ms as i64 * 1_000,
        }
    }

    pub const fn window_micros(self) -> i64 {
        self.window_micros
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bubble {
    pub side: TakerSide,
    pub start: Timestamp,
    pub end: Timestamp,
    pub first_trade_id: u64,
    pub last_trade_id: u64,
    pub trade_count: u32,
    pub size: Decimal,
    pub notional: Decimal,
    pub low: Decimal,
    pub high: Decimal,
}

impl Bubble {
    fn starting(trade: &Trade) -> Self {
        Self {
            side: trade.taker_side,
            start: trade.engine_time,
            end: trade.engine_time,
            first_trade_id: trade.venue_trade_id,
            last_trade_id: trade.venue_trade_id,
            trade_count: 1,
            size: trade.size,
            notional: trade.notional(),
            low: trade.price,
            high: trade.price,
        }
    }

    fn continues_with(&self, trade: &Trade, grouping: BubbleGrouping) -> bool {
        trade.taker_side == self.side
            && self.last_trade_id.checked_add(1) == Some(trade.venue_trade_id)
            && trade.engine_time.as_micros() - self.start.as_micros() < grouping.window_micros
    }

    fn absorb(&mut self, trade: &Trade) {
        self.end = trade.engine_time;
        self.last_trade_id = trade.venue_trade_id;
        self.trade_count = self.trade_count.saturating_add(1);
        self.size += trade.size;
        self.notional += trade.notional();
        self.low = self.low.min(trade.price);
        self.high = self.high.max(trade.price);
    }

    pub fn vwap(&self) -> Decimal {
        if self.size.is_zero() {
            self.low
        } else {
            self.notional / self.size
        }
    }
}

#[derive(Debug, Clone)]
pub struct BubbleBuilder {
    grouping: BubbleGrouping,
    open: Option<Bubble>,
}

impl BubbleBuilder {
    pub const fn new(grouping: BubbleGrouping) -> Self {
        Self {
            grouping,
            open: None,
        }
    }

    pub fn push(&mut self, trade: &Trade) -> Option<Bubble> {
        if let Some(open) = &mut self.open
            && open.continues_with(trade, self.grouping)
        {
            open.absorb(trade);
            return None;
        }
        self.open.replace(Bubble::starting(trade))
    }

    pub fn close(&mut self) -> Option<Bubble> {
        self.open.take()
    }

    pub const fn open(&self) -> Option<&Bubble> {
        self.open.as_ref()
    }
}

pub fn bubbles(trades: &[Trade], grouping: BubbleGrouping) -> Vec<Bubble> {
    let mut builder = BubbleBuilder::new(grouping);
    let mut found: Vec<Bubble> = trades.iter().filter_map(|t| builder.push(t)).collect();
    found.extend(builder.close());
    found
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LargeBubbleError {
    #[error("Large Bubble percentile must be greater than 0 and less than 100")]
    Percentile,
    #[error("Large Bubble USD floor must not be negative")]
    Floor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LargeBubbleRule {
    percentile: Decimal,
    usd_floor: Option<Decimal>,
}

impl Default for LargeBubbleRule {
    fn default() -> Self {
        Self {
            percentile: Decimal::from(99),
            usd_floor: None,
        }
    }
}

impl LargeBubbleRule {
    pub fn new(percentile: Decimal, usd_floor: Option<Decimal>) -> Result<Self, LargeBubbleError> {
        if percentile <= Decimal::ZERO || percentile >= Decimal::ONE_HUNDRED {
            return Err(LargeBubbleError::Percentile);
        }
        if usd_floor.is_some_and(|f| f.is_sign_negative()) {
            return Err(LargeBubbleError::Floor);
        }
        Ok(Self {
            percentile,
            usd_floor,
        })
    }

    pub const fn percentile(&self) -> Decimal {
        self.percentile
    }

    pub const fn usd_floor(&self) -> Option<Decimal> {
        self.usd_floor
    }

    pub fn threshold_index(&self, count: u64) -> Option<u64> {
        let last = count.checked_sub(1)?;
        let rank = (self.percentile * Decimal::from(count) / Decimal::ONE_HUNDRED)
            .floor()
            .to_u64()
            .unwrap_or(last);
        Some(rank.min(last))
    }

    pub fn threshold_offset_from_largest(&self, count: u64) -> Option<u64> {
        self.threshold_index(count).map(|index| count - 1 - index)
    }

    pub fn threshold(&self, notionals: &mut [Decimal]) -> Option<Decimal> {
        let index = usize::try_from(self.threshold_index(notionals.len() as u64)?).ok()?;
        let (_, nth, _) = notionals.select_nth_unstable(index);
        Some(*nth)
    }

    pub fn is_large(&self, notional: Decimal, threshold: Option<Decimal>) -> bool {
        threshold.is_some_and(|t| notional >= t) && self.usd_floor.is_none_or(|f| notional >= f)
    }
}
