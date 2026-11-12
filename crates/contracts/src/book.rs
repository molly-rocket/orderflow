use orderflow_domain::{BookStatus, Decimal, Level, Market, Timestamp};

use crate::convert::{ConvertError, decimal_from_proto, decimal_to_proto};
use crate::v1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookState {
    pub market: Market,
    pub time: Timestamp,
    pub status: BookStatus,
    pub bids: Vec<Level>,
    pub asks: Vec<Level>,
    pub known: Option<(Decimal, Decimal)>,
    pub best_bid: Option<Level>,
    pub best_ask: Option<Level>,
}

pub fn book_status_to_proto(status: BookStatus) -> v1::BookStatus {
    match status {
        BookStatus::Live => v1::BookStatus::Live,
        BookStatus::Resyncing => v1::BookStatus::Resyncing,
        BookStatus::Disconnected => v1::BookStatus::Disconnected,
    }
}

pub fn book_status_from_proto(status: v1::BookStatus) -> Result<BookStatus, ConvertError> {
    match status {
        v1::BookStatus::Live => Ok(BookStatus::Live),
        v1::BookStatus::Resyncing => Ok(BookStatus::Resyncing),
        v1::BookStatus::Disconnected => Ok(BookStatus::Disconnected),
        v1::BookStatus::Unspecified => Err(ConvertError::MissingField("status")),
    }
}

fn level_to_proto(level: &Level) -> Result<v1::BookLevel, ConvertError> {
    Ok(v1::BookLevel {
        price: Some(decimal_to_proto(level.price)?),
        size: Some(decimal_to_proto(level.size)?),
    })
}

fn level_from_proto(level: v1::BookLevel) -> Result<Level, ConvertError> {
    Ok(Level {
        price: decimal_from_proto(level.price.ok_or(ConvertError::MissingField("price"))?)?,
        size: decimal_from_proto(level.size.ok_or(ConvertError::MissingField("size"))?)?,
    })
}

pub fn book_state_to_proto(state: &BookState) -> Result<v1::BookState, ConvertError> {
    Ok(v1::BookState {
        market: state.market.to_string(),
        time_us: state.time.as_micros(),
        status: book_status_to_proto(state.status).into(),
        bids: state
            .bids
            .iter()
            .map(level_to_proto)
            .collect::<Result<_, _>>()?,
        asks: state
            .asks
            .iter()
            .map(level_to_proto)
            .collect::<Result<_, _>>()?,
        known_low: state
            .known
            .map(|(low, _)| decimal_to_proto(low))
            .transpose()?,
        known_high: state
            .known
            .map(|(_, high)| decimal_to_proto(high))
            .transpose()?,
        best_bid: state.best_bid.as_ref().map(level_to_proto).transpose()?,
        best_ask: state.best_ask.as_ref().map(level_to_proto).transpose()?,
    })
}

pub fn book_state_from_proto(state: v1::BookState) -> Result<BookState, ConvertError> {
    let status = book_status_from_proto(state.status())?;
    let known = match (state.known_low, state.known_high) {
        (Some(low), Some(high)) => Some((decimal_from_proto(low)?, decimal_from_proto(high)?)),
        _ => None,
    };
    Ok(BookState {
        market: state.market.parse()?,
        time: Timestamp::from_micros(state.time_us),
        status,
        bids: state
            .bids
            .into_iter()
            .map(level_from_proto)
            .collect::<Result<_, _>>()?,
        asks: state
            .asks
            .into_iter()
            .map(level_from_proto)
            .collect::<Result<_, _>>()?,
        known,
        best_bid: state.best_bid.map(level_from_proto).transpose()?,
        best_ask: state.best_ask.map(level_from_proto).transpose()?,
    })
}
