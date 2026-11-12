use rust_decimal::Decimal;

use crate::{Gap, TimeRange, Timestamp};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CvdPoint {
    pub value: Decimal,
    pub complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cvd {
    day_start: Timestamp,
    value: Decimal,
    complete: bool,
}

impl Cvd {
    pub fn opening(at: Timestamp, value_since_day_start: Decimal, complete: bool) -> Self {
        Self {
            day_start: at.trading_day_start(),
            value: value_since_day_start,
            complete,
        }
    }

    pub fn advance(&mut self, bar: TimeRange, delta: Decimal, gaps: &[Gap]) -> CvdPoint {
        let day_start = bar.start().trading_day_start();
        if day_start != self.day_start {
            *self = Self::opening(day_start, Decimal::ZERO, true);
        }
        if Gap::any_overlaps(gaps, bar) {
            self.complete = false;
        }
        self.value += delta;
        self.point()
    }

    pub const fn point(&self) -> CvdPoint {
        CvdPoint {
            value: self.value,
            complete: self.complete,
        }
    }
}
