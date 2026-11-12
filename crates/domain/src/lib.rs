//! Pure domain core for orderflow. No I/O, no async, no clocks.
//!
//! Every term follows `GLOSSARY.md`. Services adapt Venue and wire formats into
//! these types and rely on the rules here; they never re-implement them.

mod book;
mod bubble;
mod cvd;
mod footprint;
mod gap;
mod imbalance;
mod market;
mod price;
mod time;
mod trade;

pub use book::{BookSide, BookStatus, Ladder, LadderRow, Level, OrderBook};
pub use bubble::{
    Bubble, BubbleBuilder, BubbleGrouping, LargeBubbleError, LargeBubbleRule, bubbles,
};
pub use cvd::{Cvd, CvdPoint};
pub use footprint::{FootprintBar, FootprintError, FootprintRow, Ohlc, RowVolume, footprint_bars};
pub use gap::{Gap, GapReason};
pub use imbalance::{
    BarImbalances, Imbalance, ImbalanceError, ImbalanceSettings, StackedImbalance, find_imbalances,
};
pub use market::{Market, MarketIdError, MarketSpec, MarketSpecError, MarketType, Venue};
pub use price::{PriceGrouping, PriceGroupingError, PriceRow, TickSize};
pub use rust_decimal::Decimal;
pub use time::{TimeRange, Timeframe, TimeframeError, Timestamp};
pub use trade::{TakerSide, Trade};
