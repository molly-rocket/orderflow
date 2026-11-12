use std::str::FromStr;

use orderflow_domain::{Decimal, Level, Market, Timestamp};
use serde::Deserialize;

pub const DEFAULT_WS_URL: &str = "wss://fstream.binance.com/public";

pub const DEFAULT_REST_URL: &str = "https://fapi.binance.com";

pub fn stream_url(base_url: &str, markets: &[Market]) -> String {
    let streams: Vec<String> = markets
        .iter()
        .map(|m| format!("{}@depth@100ms", m.native_name().to_lowercase()))
        .collect();
    format!(
        "{}/stream?streams={}",
        base_url.trim_end_matches('/'),
        streams.join("/")
    )
}

pub fn snapshot_url(rest_url: &str, market: &Market) -> String {
    format!(
        "{}/fapi/v1/depth?symbol={}&limit=1000",
        rest_url.trim_end_matches('/'),
        market.native_name()
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepthDiff {
    pub symbol: String,
    pub event_time: Timestamp,
    pub first_id: u64,
    pub final_id: u64,
    pub prev_final_id: u64,
    pub bids: Vec<Level>,
    pub asks: Vec<Level>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub last_update_id: u64,
    pub bids: Vec<Level>,
    pub asks: Vec<Level>,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("not valid JSON for a depth message: {0}")]
    Json(#[from] serde_json::Error),
    #[error("field `{field}` is not a decimal: `{value}`")]
    Decimal { field: &'static str, value: String },
}

#[derive(Deserialize)]
struct Envelope<'a> {
    #[serde(borrow)]
    data: Option<DepthData<'a>>,
}

#[derive(Deserialize)]
struct DepthData<'a> {
    #[serde(rename = "e")]
    event: &'a str,
    #[serde(rename = "E")]
    event_time_ms: i64,
    #[serde(rename = "s")]
    symbol: &'a str,
    #[serde(rename = "U")]
    first_id: u64,
    #[serde(rename = "u")]
    final_id: u64,
    #[serde(rename = "pu")]
    prev_final_id: u64,
    #[serde(rename = "b", borrow)]
    bids: Vec<[&'a str; 2]>,
    #[serde(rename = "a", borrow)]
    asks: Vec<[&'a str; 2]>,
}

#[derive(Deserialize)]
struct SnapshotData<'a> {
    #[serde(rename = "lastUpdateId")]
    last_update_id: u64,
    #[serde(borrow)]
    bids: Vec<[&'a str; 2]>,
    #[serde(borrow)]
    asks: Vec<[&'a str; 2]>,
}

pub fn parse_frame(text: &str) -> Result<Option<DepthDiff>, ParseError> {
    let envelope: Envelope<'_> = serde_json::from_str(text)?;
    let Some(data) = envelope.data else {
        return Ok(None);
    };
    if data.event != "depthUpdate" {
        return Ok(None);
    }
    Ok(Some(DepthDiff {
        symbol: data.symbol.to_owned(),
        event_time: Timestamp::from_millis(data.event_time_ms),
        first_id: data.first_id,
        final_id: data.final_id,
        prev_final_id: data.prev_final_id,
        bids: levels("b", &data.bids)?,
        asks: levels("a", &data.asks)?,
    }))
}

pub fn parse_snapshot(text: &str) -> Result<Snapshot, ParseError> {
    let data: SnapshotData<'_> = serde_json::from_str(text)?;
    Ok(Snapshot {
        last_update_id: data.last_update_id,
        bids: levels("bids", &data.bids)?,
        asks: levels("asks", &data.asks)?,
    })
}

fn levels(field: &'static str, raw: &[[&str; 2]]) -> Result<Vec<Level>, ParseError> {
    raw.iter()
        .map(|[price, size]| {
            Ok(Level {
                price: decimal(field, price)?,
                size: decimal(field, size)?,
            })
        })
        .collect()
}

fn decimal(field: &'static str, value: &str) -> Result<Decimal, ParseError> {
    Decimal::from_str(value).map_err(|_| ParseError::Decimal {
        field,
        value: value.to_owned(),
    })
}
