use std::fmt;

const MICROS_PER_SEC: i64 = 1_000_000;
const SECS_PER_DAY: u32 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Timestamp(i64);

impl Timestamp {
    pub const fn from_micros(micros: i64) -> Self {
        Self(micros)
    }

    pub const fn from_millis(millis: i64) -> Self {
        Self(millis.saturating_mul(1_000))
    }

    pub const fn as_micros(self) -> i64 {
        self.0
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}us", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimeRange {
    start: Timestamp,
    end: Timestamp,
}

impl TimeRange {
    pub fn new(start: Timestamp, end: Timestamp) -> Option<Self> {
        (start < end).then_some(Self { start, end })
    }

    pub const fn start(self) -> Timestamp {
        self.start
    }

    pub const fn end(self) -> Timestamp {
        self.end
    }

    pub fn contains(self, t: Timestamp) -> bool {
        self.start <= t && t < self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Timeframe {
    secs: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TimeframeError {
    #[error("timeframe must be a whole number of seconds that divides 24h")]
    NotDivisorOfDay,
}

impl Timeframe {
    pub const fn from_secs(secs: u32) -> Result<Self, TimeframeError> {
        if secs == 0 || !SECS_PER_DAY.is_multiple_of(secs) {
            return Err(TimeframeError::NotDivisorOfDay);
        }
        Ok(Self { secs })
    }

    pub const fn as_secs(self) -> u32 {
        self.secs
    }

    const fn micros(self) -> i64 {
        self.secs as i64 * MICROS_PER_SEC
    }

    pub const fn bar_start(self, t: Timestamp) -> Timestamp {
        let len = self.micros();
        Timestamp(t.0.div_euclid(len) * len)
    }

    pub const fn bar_range(self, t: Timestamp) -> TimeRange {
        let start = self.bar_start(t);
        TimeRange {
            start,
            end: Timestamp(start.0 + self.micros()),
        }
    }
}
