use rust_decimal::Decimal;

use crate::{FootprintBar, PriceRow, TakerSide};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ImbalanceError {
    #[error("imbalance ratio must be at least 1 (100%)")]
    RatioBelowOne,
    #[error("a stacked imbalance needs at least 2 rows")]
    StackTooShort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImbalanceSettings {
    ratio: Decimal,
    min_stack: u8,
}

impl Default for ImbalanceSettings {
    fn default() -> Self {
        Self {
            ratio: Decimal::from(3),
            min_stack: 3,
        }
    }
}

impl ImbalanceSettings {
    pub fn new(ratio: Decimal, min_stack: u8) -> Result<Self, ImbalanceError> {
        if ratio < Decimal::ONE {
            return Err(ImbalanceError::RatioBelowOne);
        }
        if min_stack < 2 {
            return Err(ImbalanceError::StackTooShort);
        }
        Ok(Self { ratio, min_stack })
    }

    pub const fn ratio(&self) -> Decimal {
        self.ratio
    }

    pub const fn min_stack(&self) -> u8 {
        self.min_stack
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Imbalance {
    pub row: PriceRow,
    pub side: TakerSide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackedImbalance {
    pub side: TakerSide,
    pub low: PriceRow,
    pub high: PriceRow,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BarImbalances {
    pub imbalances: Vec<Imbalance>,
    pub stacks: Vec<StackedImbalance>,
}

pub fn find_imbalances(bar: &FootprintBar, settings: &ImbalanceSettings) -> BarImbalances {
    let Some((low, high)) = bar.row_span() else {
        return BarImbalances::default();
    };
    let dominates = |volume: Decimal, opposite: Decimal| {
        volume > Decimal::ZERO && volume >= settings.ratio * opposite
    };

    let mut imbalances = Vec::new();
    for here in bar.rows() {
        let row = here.row;
        if row > low && dominates(here.buy, bar.volume_at(row.below()).sell) {
            imbalances.push(Imbalance {
                row,
                side: TakerSide::Buy,
            });
        }
        if row < high && dominates(here.sell, bar.volume_at(row.above()).buy) {
            imbalances.push(Imbalance {
                row,
                side: TakerSide::Sell,
            });
        }
    }

    let stacks = [TakerSide::Buy, TakerSide::Sell]
        .into_iter()
        .flat_map(|side| stacks_for(&imbalances, side, settings.min_stack))
        .collect();
    BarImbalances { imbalances, stacks }
}

fn stacks_for(imbalances: &[Imbalance], side: TakerSide, min_stack: u8) -> Vec<StackedImbalance> {
    let mut stacks = Vec::new();
    let mut run: Option<(PriceRow, PriceRow, u32)> = None;
    let mut close = |run: Option<(PriceRow, PriceRow, u32)>| {
        if let Some((low, high, len)) = run
            && len >= u32::from(min_stack)
        {
            stacks.push(StackedImbalance { side, low, high });
        }
    };
    for imbalance in imbalances.iter().filter(|i| i.side == side) {
        run = match run {
            Some((low, high, len)) if imbalance.row == high.above() => {
                Some((low, imbalance.row, len + 1))
            }
            previous => {
                close(previous);
                Some((imbalance.row, imbalance.row, 1))
            }
        };
    }
    close(run);
    stacks
}
