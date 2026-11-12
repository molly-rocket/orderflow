use std::collections::BTreeMap;

use rust_decimal::Decimal;

use crate::{PriceGrouping, PriceRow};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BookSide {
    Bid,
    Ask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BookStatus {
    Live,
    Resyncing,
    Disconnected,
}

impl BookStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Resyncing => "resyncing",
            Self::Disconnected => "disconnected",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    pub price: Decimal,
    pub size: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderBook {
    bids: BTreeMap<Decimal, Decimal>,
    asks: BTreeMap<Decimal, Decimal>,
    known: Option<(Decimal, Decimal)>,
}

impl OrderBook {
    pub fn from_snapshot(bids: &[Level], asks: &[Level]) -> Self {
        let bids: BTreeMap<_, _> = bids
            .iter()
            .filter(|l| !l.size.is_zero())
            .map(|l| (l.price, l.size))
            .collect();
        let asks: BTreeMap<_, _> = asks
            .iter()
            .filter(|l| !l.size.is_zero())
            .map(|l| (l.price, l.size))
            .collect();
        let known = match (bids.keys().next(), asks.keys().next_back()) {
            (Some(&low), Some(&high)) => Some((low, high)),
            _ => None,
        };
        Self { bids, asks, known }
    }

    pub fn apply(&mut self, side: BookSide, price: Decimal, quantity: Decimal) {
        let levels = match side {
            BookSide::Bid => &mut self.bids,
            BookSide::Ask => &mut self.asks,
        };
        if quantity.is_zero() {
            levels.remove(&price);
        } else {
            levels.insert(price, quantity);
        }
    }

    pub fn best_bid(&self) -> Option<Level> {
        self.bids
            .iter()
            .next_back()
            .map(|(&price, &size)| Level { price, size })
    }

    pub fn best_ask(&self) -> Option<Level> {
        self.asks
            .iter()
            .next()
            .map(|(&price, &size)| Level { price, size })
    }

    pub fn mid(&self) -> Option<Decimal> {
        Some(((self.best_bid()?.price + self.best_ask()?.price) / Decimal::TWO).normalize())
    }

    pub const fn known_range(&self) -> Option<(Decimal, Decimal)> {
        self.known
    }

    pub fn levels(&self, side: BookSide) -> Vec<Level> {
        let Some((low, high)) = self.known else {
            return Vec::new();
        };
        let within = |(&price, &size): (&Decimal, &Decimal)| {
            (price >= low && price <= high).then_some(Level { price, size })
        };
        match side {
            BookSide::Bid => self.bids.iter().rev().filter_map(within).collect(),
            BookSide::Ask => self.asks.iter().filter_map(within).collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LadderRow {
    pub row: PriceRow,
    pub low: Decimal,
    pub bid: Decimal,
    pub ask: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ladder {
    pub step: Decimal,
    pub known: Option<(PriceRow, PriceRow)>,
    pub rows: Vec<LadderRow>,
}

impl Ladder {
    pub fn new(
        bids: &[Level],
        asks: &[Level],
        known: Option<(Decimal, Decimal)>,
        grouping: PriceGrouping,
    ) -> Self {
        let step = grouping.step();
        let known_rows = known.and_then(|(low, high)| known_rows(low, high, grouping));
        let Some((first, last)) = known_rows else {
            return Self {
                step,
                known: None,
                rows: Vec::new(),
            };
        };
        let mut rows: BTreeMap<PriceRow, (Decimal, Decimal)> = BTreeMap::new();
        for (levels, is_bid) in [(bids, true), (asks, false)] {
            for level in levels {
                let row = grouping.row_of(level.price);
                if row < first || row > last {
                    continue;
                }
                let entry = rows.entry(row).or_default();
                if is_bid {
                    entry.0 += level.size;
                } else {
                    entry.1 += level.size;
                }
            }
        }
        Self {
            step,
            known: Some((first, last)),
            rows: rows
                .into_iter()
                .filter(|(_, (bid, ask))| !bid.is_zero() || !ask.is_zero())
                .map(|(row, (bid, ask))| LadderRow {
                    row,
                    low: grouping.row_low(row),
                    bid,
                    ask,
                })
                .collect(),
        }
    }
}

fn known_rows(
    low: Decimal,
    high: Decimal,
    grouping: PriceGrouping,
) -> Option<(PriceRow, PriceRow)> {
    let tick = grouping.tick().get();
    let mut first = grouping.row_of(low);
    if grouping.row_low(first) < low {
        first = first.above();
    }
    let mut last = grouping.row_of(high);
    if grouping.row_low(last) + grouping.step() - tick > high {
        last = last.below();
    }
    (first <= last).then_some((first, last))
}
