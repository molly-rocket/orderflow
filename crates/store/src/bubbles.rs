use clickhouse::Row;
use orderflow_domain::{Bubble, Decimal, LargeBubbleRule, Market, TakerSide, TimeRange, Timestamp};
use serde::{Deserialize, Serialize};

use crate::{Result, Store, StoreError, decimal};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BubbleBatchRecord {
    pub market: Market,
    pub bubbles: Vec<Bubble>,
    pub stream_seq: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BubbleQuery {
    pub min_notional: Decimal,
    pub side: Option<TakerSide>,
    pub limit: usize,
}

impl BubbleQuery {
    pub const DEFAULT_LIMIT: usize = 20_000;
}

impl Default for BubbleQuery {
    fn default() -> Self {
        Self {
            min_notional: Decimal::ZERO,
            side: None,
            limit: Self::DEFAULT_LIMIT,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BubblePage {
    pub bubbles: Vec<Bubble>,
    pub truncated: bool,
}

const BUBBLE_COLUMNS: &str = "taker_side, start_us, end_us, first_trade_id, last_trade_id, \
                              trade_count, size, notional, low, high";
const IN_RANGE: &str = "market = ? AND start_us >= ? AND start_us < ?";

impl Store {
    pub async fn bubbles(
        &self,
        market: &Market,
        range: TimeRange,
        query: &BubbleQuery,
    ) -> Result<BubblePage> {
        let rows = self
            .client
            .query(&format!(
                "SELECT {BUBBLE_COLUMNS} FROM bubbles FINAL \
                 WHERE {IN_RANGE} AND notional >= toDecimal128(?, 18) \
                   AND (? = 0 OR toInt8(taker_side) = ?) \
                 ORDER BY notional DESC, start_us, first_trade_id LIMIT ?"
            ))
            .bind(market.to_string())
            .bind(range.start().as_micros())
            .bind(range.end().as_micros())
            .bind(query.min_notional.to_string())
            .bind(query.side.map_or(0, side_code))
            .bind(query.side.map_or(0, side_code))
            .bind(
                u64::try_from(query.limit)
                    .unwrap_or(u64::MAX)
                    .saturating_add(1),
            )
            .fetch_all::<BubbleRead>()
            .await?;
        let truncated = rows.len() > query.limit;
        let mut bubbles = rows
            .into_iter()
            .take(query.limit)
            .map(BubbleRead::into_bubble)
            .collect::<Result<Vec<_>>>()?;
        bubbles.sort_by_key(|b| (b.start, b.first_trade_id));
        Ok(BubblePage { bubbles, truncated })
    }

    pub async fn large_bubble_threshold(
        &self,
        market: &Market,
        window: TimeRange,
        rule: &LargeBubbleRule,
    ) -> Result<Option<Decimal>> {
        let count = self.bubble_count(market, window).await?;
        self.large_bubble_threshold_of(market, window, rule, count)
            .await
    }

    pub async fn bubble_count(&self, market: &Market, window: TimeRange) -> Result<u64> {
        Ok(self
            .client
            .query(&format!(
                "SELECT count() FROM bubbles FINAL WHERE {IN_RANGE}"
            ))
            .bind(market.to_string())
            .bind(window.start().as_micros())
            .bind(window.end().as_micros())
            .fetch_one::<u64>()
            .await?)
    }

    pub async fn large_bubble_threshold_of(
        &self,
        market: &Market,
        window: TimeRange,
        rule: &LargeBubbleRule,
        count: u64,
    ) -> Result<Option<Decimal>> {
        let Some(offset) = rule.threshold_offset_from_largest(count) else {
            return Ok(None);
        };
        let notional = self
            .client
            .query(&format!(
                "SELECT notional FROM bubbles FINAL WHERE {IN_RANGE}                  ORDER BY notional DESC LIMIT 1 OFFSET ?"
            ))
            .bind(market.to_string())
            .bind(window.start().as_micros())
            .bind(window.end().as_micros())
            .bind(offset)
            .fetch_optional::<i128>()
            .await?;
        notional.map(decimal::from_clickhouse).transpose()
    }
}

const fn side_code(side: TakerSide) -> i8 {
    match side {
        TakerSide::Buy => 1,
        TakerSide::Sell => 2,
    }
}

#[derive(Row, Serialize)]
pub(crate) struct BubbleRow<'a> {
    market: &'a str,
    first_trade_id: u64,
    last_trade_id: u64,
    start_us: i64,
    end_us: i64,
    taker_side: i8,
    trade_count: u32,
    size: i128,
    notional: i128,
    low: i128,
    high: i128,
    stream_seq: u64,
}

impl<'a> BubbleRow<'a> {
    pub(crate) fn new(market: &'a str, bubble: &Bubble, stream_seq: u64) -> Result<Self> {
        Ok(Self {
            market,
            first_trade_id: bubble.first_trade_id,
            last_trade_id: bubble.last_trade_id,
            start_us: bubble.start.as_micros(),
            end_us: bubble.end.as_micros(),
            taker_side: side_code(bubble.side),
            trade_count: bubble.trade_count,
            size: decimal::to_clickhouse(bubble.size)?,
            notional: decimal::to_clickhouse(bubble.notional)?,
            low: decimal::to_clickhouse(bubble.low)?,
            high: decimal::to_clickhouse(bubble.high)?,
            stream_seq,
        })
    }
}

#[derive(Row, Deserialize)]
struct BubbleRead {
    taker_side: i8,
    start_us: i64,
    end_us: i64,
    first_trade_id: u64,
    last_trade_id: u64,
    trade_count: u32,
    size: i128,
    notional: i128,
    low: i128,
    high: i128,
}

impl BubbleRead {
    fn into_bubble(self) -> Result<Bubble> {
        Ok(Bubble {
            side: match self.taker_side {
                1 => TakerSide::Buy,
                2 => TakerSide::Sell,
                other => return Err(StoreError::Corrupt(format!("taker side code {other}"))),
            },
            start: Timestamp::from_micros(self.start_us),
            end: Timestamp::from_micros(self.end_us),
            first_trade_id: self.first_trade_id,
            last_trade_id: self.last_trade_id,
            trade_count: self.trade_count,
            size: decimal::from_clickhouse(self.size)?,
            notional: decimal::from_clickhouse(self.notional)?,
            low: decimal::from_clickhouse(self.low)?,
            high: decimal::from_clickhouse(self.high)?,
        })
    }
}
