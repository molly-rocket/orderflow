use std::collections::BTreeMap;

use rust_decimal::Decimal;

use crate::{PriceGrouping, PriceRow, TakerSide, TimeRange, Timeframe, Trade};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FootprintError {
    #[error("trade is outside the bar's time range")]
    TradeOutsideBar,
    #[error(
        "target price grouping is not a whole multiple of the bar's grouping on the same tick size"
    )]
    GroupingDoesNotNest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RowVolume {
    pub buy: Decimal,
    pub sell: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FootprintRow {
    pub row: PriceRow,
    pub low: Decimal,
    pub buy: Decimal,
    pub sell: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FootprintBar {
    range: TimeRange,
    grouping: PriceGrouping,
    rows: BTreeMap<PriceRow, RowVolume>,
    trade_count: u64,
}

impl FootprintBar {
    pub fn new(range: TimeRange, grouping: PriceGrouping) -> Self {
        Self {
            range,
            grouping,
            rows: BTreeMap::new(),
            trade_count: 0,
        }
    }

    pub fn add(&mut self, trade: &Trade) -> Result<(), FootprintError> {
        if !self.range.contains(trade.engine_time) {
            return Err(FootprintError::TradeOutsideBar);
        }
        self.record(trade);
        Ok(())
    }

    fn record(&mut self, trade: &Trade) {
        let volume = self
            .rows
            .entry(self.grouping.row_of(trade.price))
            .or_default();
        match trade.taker_side {
            TakerSide::Buy => volume.buy += trade.size,
            TakerSide::Sell => volume.sell += trade.size,
        }
        self.trade_count += 1;
    }

    pub fn add_row_volume(&mut self, row: PriceRow, volume: RowVolume, trade_count: u64) {
        let entry = self.rows.entry(row).or_default();
        entry.buy += volume.buy;
        entry.sell += volume.sell;
        self.trade_count += trade_count;
    }

    pub const fn range(&self) -> TimeRange {
        self.range
    }

    pub const fn grouping(&self) -> PriceGrouping {
        self.grouping
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub const fn trade_count(&self) -> u64 {
        self.trade_count
    }

    pub fn rows(&self) -> impl ExactSizeIterator<Item = FootprintRow> + '_ {
        self.rows.iter().map(|(&row, v)| FootprintRow {
            row,
            low: self.grouping.row_low(row),
            buy: v.buy,
            sell: v.sell,
        })
    }


    pub fn volume_at(&self, row: PriceRow) -> RowVolume {
        self.rows.get(&row).copied().unwrap_or_default()
    }


    pub fn row_span(&self) -> Option<(PriceRow, PriceRow)> {
        Some((*self.rows.keys().next()?, *self.rows.keys().next_back()?))
    }

    pub fn buy_volume(&self) -> Decimal {
        self.rows.values().map(|v| v.buy).sum()
    }

    pub fn sell_volume(&self) -> Decimal {
        self.rows.values().map(|v| v.sell).sum()
    }

    pub fn total_volume(&self) -> Decimal {
        self.rows.values().map(|v| v.buy + v.sell).sum()
    }

    pub fn delta(&self) -> Decimal {
        self.rows.values().map(|v| v.buy - v.sell).sum()
    }

    pub fn regroup(&self, coarser: PriceGrouping) -> Result<Self, FootprintError> {
        if coarser == self.grouping {
            return Ok(self.clone());
        }
        if !self.grouping.can_coarsen_to(&coarser) {
            return Err(FootprintError::GroupingDoesNotNest);
        }
        let mut out = Self::new(self.range, coarser);
        for (&row, &volume) in &self.rows {
            out.add_row_volume(self.grouping.coarsen_row(row, &coarser), volume, 0);
        }
        out.trade_count = self.trade_count;
        Ok(out)
    }
}


pub fn footprint_bars(
    trades: &[Trade],
    timeframe: Timeframe,
    grouping: PriceGrouping,
) -> Vec<FootprintBar> {
    let mut bars: Vec<FootprintBar> = Vec::new();
    for trade in trades {
        match bars.last_mut() {
            Some(bar) if bar.range.contains(trade.engine_time) => bar.record(trade),
            _ => {
                let mut bar = FootprintBar::new(timeframe.bar_range(trade.engine_time), grouping);
                bar.record(trade);
                bars.push(bar);
            }
        }
    }
    bars
}
