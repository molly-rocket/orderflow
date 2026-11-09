use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TickSize(Decimal);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PriceGroupingError {
    #[error("tick size must be positive")]
    NonPositiveTick,
    #[error("price grouping multiple must be at least 1")]
    ZeroMultiple,
    #[error("price grouping must be a whole multiple of the tick size")]
    NotTickMultiple,
}

impl TickSize {
    pub fn new(tick: Decimal) -> Result<Self, PriceGroupingError> {
        if tick <= Decimal::ZERO {
            return Err(PriceGroupingError::NonPositiveTick);
        }
        Ok(Self(tick.normalize()))
    }

    pub const fn get(self) -> Decimal {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PriceGrouping {
    tick: TickSize,
    multiple: u32,
    step: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PriceRow(i64);

impl PriceRow {
    pub const fn from_index(index: i64) -> Self {
        Self(index)
    }

    pub const fn index(self) -> i64 {
        self.0
    }

    #[must_use]
    pub const fn above(self) -> Self {
        Self(self.0 + 1)
    }

    #[must_use]
    pub const fn below(self) -> Self {
        Self(self.0 - 1)
    }
}

impl PriceGrouping {
    pub fn new(tick: TickSize, multiple: u32) -> Result<Self, PriceGroupingError> {
        if multiple == 0 {
            return Err(PriceGroupingError::ZeroMultiple);
        }
        Ok(Self {
            tick,
            multiple,
            step: (tick.0 * Decimal::from(multiple)).normalize(),
        })
    }

    pub fn from_step(tick: TickSize, step: Decimal) -> Result<Self, PriceGroupingError> {
        if step <= Decimal::ZERO || !(step % tick.0).is_zero() {
            return Err(PriceGroupingError::NotTickMultiple);
        }
        let multiple = (step / tick.0)
            .to_u32()
            .ok_or(PriceGroupingError::NotTickMultiple)?;
        Self::new(tick, multiple)
    }

    pub const fn tick(&self) -> TickSize {
        self.tick
    }

    pub const fn multiple(&self) -> u32 {
        self.multiple
    }

    pub const fn step(&self) -> Decimal {
        self.step
    }

    pub fn row_of(&self, price: Decimal) -> PriceRow {
        let index = (price / self.step)
            .floor()
            .to_i64()
            .expect("price row index fits in i64");
        PriceRow(index)
    }

    pub fn row_low(&self, row: PriceRow) -> Decimal {
        (Decimal::from(row.0) * self.step).normalize()
    }

    pub fn can_coarsen_to(&self, coarser: &Self) -> bool {
        self.tick == coarser.tick
            && coarser.multiple > self.multiple
            && coarser.multiple.is_multiple_of(self.multiple)
    }

    pub fn coarsen_row(&self, row: PriceRow, coarser: &Self) -> PriceRow {
        let factor = i64::from(coarser.multiple / self.multiple);
        PriceRow(row.0.div_euclid(factor))
    }
}
