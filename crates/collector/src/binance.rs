use std::str::FromStr;

use orderflow_domain::{Decimal, Market, TakerSide, Timestamp, Trade};
use serde::Deserialize;

pub const DEFAULT_BASE_URL: &str = "wss://fstream.binance.com/market";

pub fn stream_url(base_url: &str, markets: &[Market]) -> String {
    let streams: Vec<String> = markets
        .iter()
        .map(|m| format!("{}@aggTrade", m.native_name().to_lowercase()))
        .collect();
    format!(
        "{}/stream?streams={}",
        base_url.trim_end_matches('/'),
        streams.join("/")
    )
}

#[derive(Debug, PartialEq, Eq)]
pub enum Frame<'a> {
    AggTrade { symbol: &'a str, trade: Trade },
    Other,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("frame is not valid JSON for a combined stream: {0}")]
    Json(#[from] serde_json::Error),
    #[error("field `{field}` is not a decimal: `{value}`")]
    Decimal { field: &'static str, value: String },
}

#[derive(Deserialize)]
struct Envelope<'a> {
    #[serde(borrow)]
    data: Option<AggTradeData<'a>>,
}

#[derive(Deserialize)]
struct AggTradeData<'a> {
    #[serde(rename = "e")]
    event: &'a str,
    #[serde(rename = "s")]
    symbol: &'a str,
    #[serde(rename = "a")]
    aggregate_id: Option<u64>,
    #[serde(rename = "p")]
    price: Option<&'a str>,
    #[serde(rename = "q")]
    quantity: Option<&'a str>,
    #[serde(rename = "T")]
    trade_time_ms: Option<i64>,
    #[serde(rename = "m")]
    buyer_is_maker: Option<bool>,
}

pub fn parse_frame(text: &str) -> Result<Frame<'_>, ParseError> {
    let envelope: Envelope<'_> = serde_json::from_str(text)?;
    let Some(data) = envelope.data else {
        return Ok(Frame::Other);
    };
    let (
        "aggTrade",
        Some(aggregate_id),
        Some(price),
        Some(quantity),
        Some(trade_time_ms),
        Some(buyer_is_maker),
    ) = (
        data.event,
        data.aggregate_id,
        data.price,
        data.quantity,
        data.trade_time_ms,
        data.buyer_is_maker,
    )
    else {
        return Ok(Frame::Other);
    };
    Ok(Frame::AggTrade {
        symbol: data.symbol,
        trade: Trade {
            venue_trade_id: aggregate_id,
            engine_time: Timestamp::from_millis(trade_time_ms),
            price: decimal("p", price)?,
            size: decimal("q", quantity)?,
            taker_side: if buyer_is_maker {
                TakerSide::Sell
            } else {
                TakerSide::Buy
            },
            taker_order_id: None,
        },
    })
}

fn decimal(field: &'static str, value: &str) -> Result<Decimal, ParseError> {
    Decimal::from_str(value).map_err(|_| ParseError::Decimal {
        field,
        value: value.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_live_usdm_agg_trade_frame() {
        // Verbatim from the 2026-11-09 capture.
        let text = r#"{"stream":"btcusdt@aggTrade","data":{"e":"aggTrade","E":1794179696349,"a":3512548202,"s":"BTCUSDT","p":"75191.30","q":"0.086","nq":"0.086","f":8246859258,"l":8246859258,"T":1794179696319,"m":true,"st":1}}"#;

        let Frame::AggTrade { symbol, trade } = parse_frame(text).unwrap() else {
            panic!("expected an aggTrade");
        };
        assert_eq!(symbol, "BTCUSDT");
        assert_eq!(trade.venue_trade_id, 3_512_548_202);
        assert_eq!(trade.engine_time, Timestamp::from_millis(1_794_179_696_319));
        assert_eq!(trade.price.to_string(), "75191.30");
        assert_eq!(trade.size.to_string(), "0.086");
        assert_eq!(trade.taker_side, TakerSide::Sell);
    }

    #[test]
    fn non_trade_frames_are_other_and_garbage_is_an_error() {
        assert_eq!(
            parse_frame(r#"{"result":null,"id":1}"#).unwrap(),
            Frame::Other
        );
        assert!(parse_frame("not json").is_err());
        assert!(
            parse_frame(
                r#"{"data":{"e":"aggTrade","s":"X","a":1,"p":"x","q":"1","T":1,"m":true}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn stream_url_lists_every_market_in_lowercase() {
        let markets: Vec<Market> = ["binance:perp:BTCUSDT", "binance:perp:ETHUSDT"]
            .iter()
            .map(|m| m.parse().unwrap())
            .collect();
        assert_eq!(
            stream_url("wss://x/market/", &markets),
            "wss://x/market/stream?streams=btcusdt@aggTrade/ethusdt@aggTrade"
        );
    }
}
