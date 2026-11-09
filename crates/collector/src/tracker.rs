use orderflow_domain::{GapReason, Market, Timestamp, Trade};

use crate::checkpoint::{Checkpoint, OpenGap};
use crate::sink::Record;

#[derive(Debug)]
pub(crate) struct Tracker {
    market: Market,
    last: Option<(u64, Timestamp)>,
    open_gap: Option<OpenGap>,
    dirty: bool,
}

impl Tracker {
    pub(crate) fn resume(market: Market, checkpoint: Option<Checkpoint>) -> (Self, Option<Record>) {
        let mut tracker = Self {
            market,
            last: None,
            open_gap: None,
            dirty: false,
        };
        let Some(checkpoint) = checkpoint else {
            return (tracker, None);
        };
        tracker.last = Some((
            checkpoint.last_trade_id,
            Timestamp::from_micros(checkpoint.last_engine_time_us),
        ));
        if let Some(open) = checkpoint.open_gap {
            tracker.open_gap = Some(open);
            return (tracker, None);
        }
        let opened = tracker.lapse(GapReason::RecorderDowntime);
        (tracker, opened)
    }

    pub(crate) fn market(&self) -> &Market {
        &self.market
    }

    pub(crate) fn observe(&mut self, trade: &Trade, out: &mut Vec<Record>) -> bool {
        if let Some((last_id, _)) = self.last
            && trade.venue_trade_id <= last_id
        {
            return false;
        }
        if let Some(gap) = self.open_gap.take() {
            let end = trade.engine_time.max(gap.start());
            out.push(Record::GapClosed {
                gap_id: gap.gap_id,
                market: self.market.clone(),
                end,
            });
        } else if let Some((last_id, last_time)) = self.last
            && trade.venue_trade_id != last_id + 1
        {
            let gap_id = gap_id(&self.market, last_id, GapReason::SequenceBreak);
            out.push(Record::GapOpened {
                gap_id: gap_id.clone(),
                market: self.market.clone(),
                start: last_time,
                reason: GapReason::SequenceBreak,
            });
            out.push(Record::GapClosed {
                gap_id,
                market: self.market.clone(),
                end: trade.engine_time.max(last_time),
            });
        }
        self.last = Some((trade.venue_trade_id, trade.engine_time));
        self.dirty = true;
        true
    }

    pub(crate) fn lapse(&mut self, reason: GapReason) -> Option<Record> {
        if self.open_gap.is_some() {
            return None;
        }
        let (last_id, last_time) = self.last?;
        let gap_id = gap_id(&self.market, last_id, reason);
        self.open_gap = Some(OpenGap {
            gap_id: gap_id.clone(),
            start_us: last_time.as_micros(),
            reason: reason.into(),
        });
        self.dirty = true;
        Some(Record::GapOpened {
            gap_id,
            market: self.market.clone(),
            start: last_time,
            reason,
        })
    }

    pub(crate) fn take_checkpoint(&mut self) -> Option<Checkpoint> {
        if !self.dirty {
            return None;
        }
        let (last_trade_id, last_time) = self.last?;
        self.dirty = false;
        Some(Checkpoint {
            last_trade_id,
            last_engine_time_us: last_time.as_micros(),
            open_gap: self.open_gap.clone(),
        })
    }
}

fn gap_id(market: &Market, last_trade_id: u64, reason: GapReason) -> String {
    format!("{market}/{last_trade_id}/{reason:?}")
}
