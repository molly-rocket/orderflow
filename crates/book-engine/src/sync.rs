use orderflow_domain::{BookSide, BookStatus, Decimal, OrderBook, Timestamp};

use crate::binance::{DepthDiff, Snapshot};

const RECENTRE_FLOOR_MICROS: i64 = 10_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    FetchSnapshot,
}

#[derive(Debug)]
struct Aligning {
    diffs: Vec<DepthDiff>,
    pending: Option<Snapshot>,
}

#[derive(Debug)]
enum State {
    Disconnected,
    Resyncing(Aligning),
    Live {
        book: OrderBook,
        last_final_id: u64,
        recentre: Option<Aligning>,
    },
}

enum Alignment {
    Live(OrderBook, u64),
    TooOld,
    Pending,
}

#[derive(Debug)]
pub struct BookSync {
    max_buffer: usize,
    state: State,
    last_request: Option<Timestamp>,
}

impl BookSync {
    pub fn new(max_buffer: usize) -> Self {
        Self {
            max_buffer: max_buffer.max(1),
            state: State::Disconnected,
            last_request: None,
        }
    }

    pub fn status(&self) -> BookStatus {
        match self.state {
            State::Disconnected => BookStatus::Disconnected,
            State::Resyncing(_) => BookStatus::Resyncing,
            State::Live { .. } => BookStatus::Live,
        }
    }

    pub fn book(&self) -> Option<&OrderBook> {
        match &self.state {
            State::Live { book, .. } => Some(book),
            _ => None,
        }
    }

    pub fn on_diff(&mut self, diff: DepthDiff, now: Timestamp) -> Option<Action> {
        let request_allowed = self.request_allowed(now);
        match &mut self.state {
            State::Disconnected => Some(self.restart(diff, now)),
            State::Resyncing(aligning) => {
                if let Some(snapshot) = aligning.pending.take() {
                    return match align(&snapshot, std::slice::from_ref(&diff)) {
                        Alignment::Live(book, last_final_id) => {
                            self.state = State::Live {
                                book,
                                last_final_id,
                                recentre: None,
                            };
                            None
                        }
                        Alignment::TooOld => Some(self.restart(diff, now)),
                        Alignment::Pending => {
                            aligning.pending = Some(snapshot);
                            None
                        }
                    };
                }
                if aligning.diffs.len() >= self.max_buffer {
                    return Some(self.restart(diff, now));
                }
                aligning.diffs.push(diff);
                None
            }
            State::Live {
                book,
                last_final_id,
                recentre,
            } => {
                if diff.prev_final_id != *last_final_id {
                    return Some(self.restart(diff, now));
                }
                apply(book, &diff);
                *last_final_id = diff.final_id;
                if let Some(aligning) = recentre {
                    if let Some(snapshot) = aligning.pending.take() {
                        match align(&snapshot, &[diff]) {
                            Alignment::Live(fresh, id) => {
                                *book = fresh;
                                *last_final_id = id;
                                *recentre = None;
                            }
                            Alignment::TooOld => *recentre = None,
                            Alignment::Pending => aligning.pending = Some(snapshot),
                        }
                    } else if aligning.diffs.len() >= self.max_buffer {
                        *recentre = None;
                    } else {
                        aligning.diffs.push(diff);
                    }
                    return None;
                }
                if request_allowed && needs_recentre(book) {
                    *recentre = Some(Aligning {
                        diffs: Vec::new(),
                        pending: None,
                    });
                    return Some(self.request(now));
                }
                None
            }
        }
    }

    pub fn on_snapshot(&mut self, snapshot: Snapshot, now: Timestamp) -> Option<Action> {
        match &mut self.state {
            State::Disconnected | State::Live { recentre: None, .. } => None,
            State::Resyncing(aligning) => match align(&snapshot, &aligning.diffs) {
                Alignment::Live(book, last_final_id) => {
                    self.state = State::Live {
                        book,
                        last_final_id,
                        recentre: None,
                    };
                    None
                }
                Alignment::TooOld => {
                    aligning.diffs.clear();
                    Some(self.request(now))
                }
                Alignment::Pending => {
                    aligning.pending = Some(snapshot);
                    None
                }
            },
            State::Live {
                book,
                last_final_id,
                recentre: Some(aligning),
            } => {
                match align(&snapshot, &aligning.diffs) {
                    Alignment::Live(fresh, id) => {
                        *book = fresh;
                        *last_final_id = id;
                        self.clear_recentre();
                    }
                    Alignment::TooOld => self.clear_recentre(),
                    Alignment::Pending => aligning.pending = Some(snapshot),
                }
                None
            }
        }
    }

    pub fn on_snapshot_failed(&mut self) -> Option<Action> {
        match &mut self.state {
            State::Resyncing(_) => Some(Action::FetchSnapshot),
            State::Live { recentre, .. } => {
                *recentre = None;
                None
            }
            State::Disconnected => None,
        }
    }

    pub fn on_disconnect(&mut self) {
        self.state = State::Disconnected;
    }

    fn clear_recentre(&mut self) {
        if let State::Live { recentre, .. } = &mut self.state {
            *recentre = None;
        }
    }

    fn restart(&mut self, diff: DepthDiff, now: Timestamp) -> Action {
        self.state = State::Resyncing(Aligning {
            diffs: vec![diff],
            pending: None,
        });
        self.request(now)
    }

    fn request(&mut self, now: Timestamp) -> Action {
        self.last_request = Some(now);
        Action::FetchSnapshot
    }

    fn request_allowed(&self, now: Timestamp) -> bool {
        self.last_request.is_none_or(|last| {
            now.as_micros().saturating_sub(last.as_micros()) >= RECENTRE_FLOOR_MICROS
        })
    }
}

fn align(snapshot: &Snapshot, diffs: &[DepthDiff]) -> Alignment {
    let mut remaining = diffs
        .iter()
        .filter(|d| d.final_id >= snapshot.last_update_id)
        .peekable();
    let Some(first) = remaining.peek() else {
        return Alignment::Pending;
    };
    if first.first_id > snapshot.last_update_id {
        return Alignment::TooOld;
    }
    let mut book = OrderBook::from_snapshot(&snapshot.bids, &snapshot.asks);
    let mut last_final_id = None;
    for diff in remaining {
        if last_final_id.is_some_and(|last| diff.prev_final_id != last) {
            return Alignment::TooOld;
        }
        apply(&mut book, diff);
        last_final_id = Some(diff.final_id);
    }
    last_final_id.map_or(Alignment::Pending, |id| Alignment::Live(book, id))
}

fn apply(book: &mut OrderBook, diff: &DepthDiff) {
    for level in &diff.bids {
        book.apply(BookSide::Bid, level.price, level.size);
    }
    for level in &diff.asks {
        book.apply(BookSide::Ask, level.price, level.size);
    }
}

fn needs_recentre(book: &OrderBook) -> bool {
    let (Some(mid), Some((low, high))) = (book.mid(), book.known_range()) else {
        return false;
    };
    if mid < low || mid > high {
        return true;
    }
    let centre = (low + high) / Decimal::TWO;
    let quarter = (high - low) / Decimal::from(4);
    (mid - centre).abs() > quarter
}
