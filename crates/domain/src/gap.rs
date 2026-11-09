use crate::{TimeRange, Timestamp};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GapReason {
    Disconnect,
    SequenceBreak,
    BookResync,
    RecorderDowntime,
    StorageOutage,
}

impl GapReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disconnect => "disconnect",
            Self::SequenceBreak => "sequence_break",
            Self::BookResync => "book_resync",
            Self::RecorderDowntime => "recorder_downtime",
            Self::StorageOutage => "storage_outage",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Gap {
    start: Timestamp,
    end: Option<Timestamp>,
    reason: GapReason,
}

impl Gap {
    pub const fn open(start: Timestamp, reason: GapReason) -> Self {
        Self {
            start,
            end: None,
            reason,
        }
    }

    pub fn closed(start: Timestamp, end: Timestamp, reason: GapReason) -> Option<Self> {
        (end >= start).then_some(Self {
            start,
            end: Some(end),
            reason,
        })
    }

    #[must_use]
    pub fn close(self, end: Timestamp) -> Option<Self> {
        Self::closed(self.start, end, self.reason)
    }

    pub const fn start(&self) -> Timestamp {
        self.start
    }

    pub const fn end(&self) -> Option<Timestamp> {
        self.end
    }

    pub const fn reason(&self) -> GapReason {
        self.reason
    }

    pub const fn is_open(&self) -> bool {
        self.end.is_none()
    }

    pub fn overlaps(&self, range: TimeRange) -> bool {
        self.start < range.end() && self.end.is_none_or(|end| end > range.start())
    }

    pub fn any_overlaps(gaps: &[Self], range: TimeRange) -> bool {
        gaps.iter().any(|gap| gap.overlaps(range))
    }
}
