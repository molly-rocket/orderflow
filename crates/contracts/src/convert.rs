use orderflow_domain::{
    Bubble, Decimal, GapReason, Market, MarketIdError, TakerSide, Timestamp, Trade,
};

use crate::v1;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConvertError {
    #[error("decimal does not fit the wire format (i64 mantissa, scale <= 28)")]
    DecimalOutOfRange,
    #[error("required field `{0}` is missing or unspecified")]
    MissingField(&'static str),
    #[error("invalid market id: {0}")]
    Market(#[from] MarketIdError),
}

pub fn decimal_to_proto(value: Decimal) -> Result<v1::Decimal, ConvertError> {
    let mantissa = i64::try_from(value.mantissa()).map_err(|_| ConvertError::DecimalOutOfRange)?;
    Ok(v1::Decimal {
        mantissa,
        scale: value.scale(),
    })
}

pub fn decimal_from_proto(value: v1::Decimal) -> Result<Decimal, ConvertError> {
    Decimal::try_from_i128_with_scale(i128::from(value.mantissa), value.scale)
        .map_err(|_| ConvertError::DecimalOutOfRange)
}

pub fn taker_side_to_proto(side: TakerSide) -> v1::TakerSide {
    match side {
        TakerSide::Buy => v1::TakerSide::Buy,
        TakerSide::Sell => v1::TakerSide::Sell,
    }
}

pub fn taker_side_from_proto(side: v1::TakerSide) -> Result<TakerSide, ConvertError> {
    match side {
        v1::TakerSide::Buy => Ok(TakerSide::Buy),
        v1::TakerSide::Sell => Ok(TakerSide::Sell),
        v1::TakerSide::Unspecified => Err(ConvertError::MissingField("taker_side")),
    }
}

pub fn gap_reason_to_proto(reason: GapReason) -> v1::GapReason {
    match reason {
        GapReason::Disconnect => v1::GapReason::Disconnect,
        GapReason::SequenceBreak => v1::GapReason::SequenceBreak,
        GapReason::BookResync => v1::GapReason::BookResync,
        GapReason::RecorderDowntime => v1::GapReason::RecorderDowntime,
        GapReason::StorageOutage => v1::GapReason::StorageOutage,
    }
}

pub fn gap_reason_from_proto(reason: v1::GapReason) -> Result<GapReason, ConvertError> {
    match reason {
        v1::GapReason::Disconnect => Ok(GapReason::Disconnect),
        v1::GapReason::SequenceBreak => Ok(GapReason::SequenceBreak),
        v1::GapReason::BookResync => Ok(GapReason::BookResync),
        v1::GapReason::RecorderDowntime => Ok(GapReason::RecorderDowntime),
        v1::GapReason::StorageOutage => Ok(GapReason::StorageOutage),
        v1::GapReason::Unspecified => Err(ConvertError::MissingField("reason")),
    }
}

pub fn trade_to_proto(trade: &Trade) -> Result<v1::Trade, ConvertError> {
    Ok(v1::Trade {
        venue_trade_id: trade.venue_trade_id,
        engine_time_us: trade.engine_time.as_micros(),
        price: Some(decimal_to_proto(trade.price)?),
        size: Some(decimal_to_proto(trade.size)?),
        taker_side: taker_side_to_proto(trade.taker_side).into(),
        taker_order_id: trade.taker_order_id.as_deref().map(str::to_owned),
    })
}

pub fn trade_from_proto(trade: v1::Trade) -> Result<Trade, ConvertError> {
    Ok(Trade {
        venue_trade_id: trade.venue_trade_id,
        engine_time: Timestamp::from_micros(trade.engine_time_us),
        price: decimal_from_proto(trade.price.ok_or(ConvertError::MissingField("price"))?)?,
        size: decimal_from_proto(trade.size.ok_or(ConvertError::MissingField("size"))?)?,
        taker_side: taker_side_from_proto(trade.taker_side())?,
        taker_order_id: trade.taker_order_id.map(String::into_boxed_str),
    })
}

pub fn trade_batch_to_proto(
    market: &Market,
    trades: &[Trade],
) -> Result<v1::TradeBatch, ConvertError> {
    Ok(v1::TradeBatch {
        market: market.to_string(),
        trades: trades
            .iter()
            .map(trade_to_proto)
            .collect::<Result<_, _>>()?,
    })
}

pub fn trade_batch_from_proto(batch: v1::TradeBatch) -> Result<(Market, Vec<Trade>), ConvertError> {
    let market = batch.market.parse()?;
    let trades = batch
        .trades
        .into_iter()
        .map(trade_from_proto)
        .collect::<Result<_, _>>()?;
    Ok((market, trades))
}

pub fn bubble_to_proto(bubble: &Bubble) -> Result<v1::Bubble, ConvertError> {
    Ok(v1::Bubble {
        taker_side: taker_side_to_proto(bubble.side).into(),
        start_time_us: bubble.start.as_micros(),
        end_time_us: bubble.end.as_micros(),
        first_trade_id: bubble.first_trade_id,
        last_trade_id: bubble.last_trade_id,
        trade_count: bubble.trade_count,
        size: Some(decimal_to_proto(bubble.size)?),
        notional: Some(decimal_to_proto(bubble.notional)?),
        low: Some(decimal_to_proto(bubble.low)?),
        high: Some(decimal_to_proto(bubble.high)?),
    })
}

pub fn bubble_from_proto(bubble: v1::Bubble) -> Result<Bubble, ConvertError> {
    let decimal = |value: Option<v1::Decimal>, field| {
        decimal_from_proto(value.ok_or(ConvertError::MissingField(field))?)
    };
    Ok(Bubble {
        side: taker_side_from_proto(bubble.taker_side())?,
        start: Timestamp::from_micros(bubble.start_time_us),
        end: Timestamp::from_micros(bubble.end_time_us),
        first_trade_id: bubble.first_trade_id,
        last_trade_id: bubble.last_trade_id,
        trade_count: bubble.trade_count,
        size: decimal(bubble.size, "size")?,
        notional: decimal(bubble.notional, "notional")?,
        low: decimal(bubble.low, "low")?,
        high: decimal(bubble.high, "high")?,
    })
}
