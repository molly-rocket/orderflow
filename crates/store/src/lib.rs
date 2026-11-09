mod decimal;
mod migrate;

use clickhouse::{Client, Row};
use orderflow_domain::{
    FootprintBar, Gap, GapReason, Market, PriceGrouping, RowVolume, TakerSide, TimeRange,
    Timeframe, Timestamp, Trade,
};
use serde::{Deserialize, Serialize};

pub use decimal::PrecisionError;

const MICROS_PER_MINUTE: i64 = 60_000_000;

#[derive(Debug, Clone)]
pub struct StoreConfig {
    pub url: String,
    pub database: String,
    pub user: String,
    pub password: String,
}

impl StoreConfig {
    pub fn local() -> Self {
        Self {
            url: "http://127.0.0.1:8123".into(),
            database: "orderflow".into(),
            user: "orderflow".into(),
            password: "orderflow".into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("clickhouse: {0}")]
    ClickHouse(#[from] clickhouse::error::Error),
    #[error(transparent)]
    Precision(#[from] PrecisionError),
    #[error("timeframe must be a whole number of minutes (footprint is stored per minute)")]
    UnsupportedTimeframe,
    #[error("invalid database name `{0}`")]
    InvalidDatabase(String),
    #[error("stored data is inconsistent: {0}")]
    Corrupt(String),
}

impl StoreError {
    pub fn is_unavailable(&self) -> bool {
        matches!(
            self,
            Self::ClickHouse(
                clickhouse::error::Error::Network(_) | clickhouse::error::Error::TimedOut
            )
        )
    }
}

pub type Result<T, E = StoreError> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeBatchRecord {
    pub market: Market,
    pub trades: Vec<Trade>,
    pub stream_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GapEvent {
    Opened {
        market: Market,
        gap_id: String,
        start: Timestamp,
        reason: GapReason,
    },
    Closed {
        market: Market,
        gap_id: String,
        end: Timestamp,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WriteBatch {
    pub trade_batches: Vec<TradeBatchRecord>,
    pub gap_events: Vec<(GapEvent, u64)>,
}

impl WriteBatch {
    pub fn is_empty(&self) -> bool {
        self.trade_batches.is_empty() && self.gap_events.is_empty()
    }

    pub fn trade_count(&self) -> usize {
        self.trade_batches.iter().map(|b| b.trades.len()).sum()
    }
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestTrade {
    pub market: Market,
    pub venue_trade_id: u64,
    pub engine_time: Timestamp,
}

#[derive(Debug, Clone)]
pub struct Store {
    client: Client,
    database: String,
}

impl Store {
    pub async fn connect(config: &StoreConfig) -> Result<Self> {
        if config.database.is_empty()
            || !config
                .database
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(StoreError::InvalidDatabase(config.database.clone()));
        }
        let server = Client::default()
            .with_url(&config.url)
            .with_user(&config.user)
            .with_password(&config.password);
        server
            .query(&format!(
                "CREATE DATABASE IF NOT EXISTS `{}`",
                config.database
            ))
            .execute()
            .await?;
        let client = server.with_database(&config.database);
        migrate::apply(&client).await?;
        Ok(Self {
            client,
            database: config.database.clone(),
        })
    }

    pub async fn drop_database_for_tests(&self) -> Result<()> {
        self.client
            .query(&format!("DROP DATABASE IF EXISTS `{}`", self.database))
            .execute()
            .await?;
        Ok(())
    }

    pub async fn write(&self, batch: &WriteBatch) -> Result<()> {
        if !batch.gap_events.is_empty() {
            let mut insert = self.client.insert::<GapEventRow<'_>>("gap_events").await?;
            for (event, seq) in &batch.gap_events {
                insert.write(&GapEventRow::new(event, *seq)).await?;
            }
            insert.end().await?;
        }
        if batch.trade_batches.iter().any(|b| !b.trades.is_empty()) {
            let mut rows = Vec::with_capacity(batch.trade_count());
            let markets: Vec<String> = batch
                .trade_batches
                .iter()
                .map(|b| b.market.to_string())
                .collect();
            for (record, market) in batch.trade_batches.iter().zip(&markets) {
                for trade in &record.trades {
                    rows.push(TradeRow::new(market, trade, record.stream_seq)?);
                }
            }
            let mut insert = self.client.insert::<TradeRow<'_>>("trades").await?;
            for row in &rows {
                insert.write(row).await?;
            }
            insert.end().await?;
        }
        Ok(())
    }

    pub async fn high_water_mark(&self) -> Result<u64> {
        Ok(self
            .client
            .query("SELECT max(stream_seq) FROM trades")
            .fetch_one::<u64>()
            .await?)
    }

    pub async fn recording_horizon(&self, market: &Market) -> Result<Option<Timestamp>> {
        let first = self
            .client
            .query("SELECT minOrNull(engine_time_us) FROM trades WHERE market = ?")
            .bind(market.to_string())
            .fetch_one::<Option<i64>>()
            .await?;
        Ok(first.map(Timestamp::from_micros))
    }

    pub async fn markets(&self) -> Result<Vec<Market>> {
        let ids = self
            .client
            .query("SELECT DISTINCT market FROM trades ORDER BY market")
            .fetch_all::<String>()
            .await?;
        ids.iter()
            .map(|id| {
                id.parse()
                    .map_err(|e| StoreError::Corrupt(format!("market `{id}`: {e}")))
            })
            .collect()
    }

    pub async fn latest_trades(&self) -> Result<Vec<LatestTrade>> {
        let rows = self
            .client
            .query(
                "SELECT market, argMax(venue_trade_id, engine_time_us), max(engine_time_us) \
                 FROM trades GROUP BY market ORDER BY market",
            )
            .fetch_all::<(String, u64, i64)>()
            .await?;
        rows.into_iter()
            .map(|(id, venue_trade_id, time)| {
                Ok(LatestTrade {
                    market: id
                        .parse()
                        .map_err(|e| StoreError::Corrupt(format!("market `{id}`: {e}")))?,
                    venue_trade_id,
                    engine_time: Timestamp::from_micros(time),
                })
            })
            .collect()
    }

    pub async fn footprint_bars(
        &self,
        market: &Market,
        range: TimeRange,
        timeframe: Timeframe,
        grouping: PriceGrouping,
    ) -> Result<Vec<FootprintBar>> {
        let timeframe_us = i64::from(timeframe.as_secs()) * 1_000_000;
        if timeframe_us % MICROS_PER_MINUTE != 0 {
            return Err(StoreError::UnsupportedTimeframe);
        }
        let cells = self
            .client
            .query(
                "SELECT intDiv(bar_start_us, ?) * ? AS bar, price, \
                        sum(buy_volume) AS buy, sum(sell_volume) AS sell, sum(trade_count) AS trades \
                 FROM footprint_1m \
                 WHERE market = ? AND bar_start_us >= ? AND bar_start_us < ? \
                 GROUP BY bar, price ORDER BY bar, price",
            )
            .bind(timeframe_us)
            .bind(timeframe_us)
            .bind(market.to_string())
            .bind(timeframe.bar_start(range.start()).as_micros())
            .bind(range.end().as_micros())
            .fetch_all::<FootprintCell>()
            .await?;

        let mut bars: Vec<FootprintBar> = Vec::new();
        for cell in cells {
            let start = Timestamp::from_micros(cell.bar);
            let row = grouping.row_of(decimal::from_clickhouse(cell.price)?);
            let volume = RowVolume {
                buy: decimal::from_clickhouse(cell.buy)?,
                sell: decimal::from_clickhouse(cell.sell)?,
            };
            match bars.last_mut() {
                Some(bar) if bar.range().start() == start => {
                    bar.add_row_volume(row, volume, cell.trades);
                }
                _ => {
                    let mut bar = FootprintBar::new(timeframe.bar_range(start), grouping);
                    bar.add_row_volume(row, volume, cell.trades);
                    bars.push(bar);
                }
            }
        }
        Ok(bars)
    }

    pub async fn gaps(&self, market: &Market, range: TimeRange) -> Result<Vec<Gap>> {
        Ok(self
            .gaps_with_ids(market, range)
            .await?
            .into_iter()
            .map(|g| g.gap)
            .collect())
    }

    pub async fn gaps_with_ids(&self, market: &Market, range: TimeRange) -> Result<Vec<StoredGap>> {
        let rows = self
            .client
            .query(
                "SELECT gap_id, minIf(time_us, kind = 'open') AS start_us, \
                        maxIf(time_us, kind = 'close') AS end_us, \
                        countIf(kind = 'close') > 0 AS closed, \
                        toInt8(anyIf(reason, kind = 'open')) AS reason \
                 FROM gap_events WHERE market = ? \
                 GROUP BY gap_id \
                 HAVING countIf(kind = 'open') > 0 AND start_us < ? AND (NOT closed OR end_us > ?) \
                 ORDER BY start_us, gap_id",
            )
            .bind(market.to_string())
            .bind(range.end().as_micros())
            .bind(range.start().as_micros())
            .fetch_all::<GapRow>()
            .await?;
        rows.into_iter().map(GapRow::into_gap).collect()
    }
}

#[derive(Row, Serialize)]
struct TradeRow<'a> {
    market: &'a str,
    venue_trade_id: u64,
    engine_time_us: i64,
    price: i128,
    size: i128,
    taker_side: i8,
    taker_order_id: Option<&'a str>,
    stream_seq: u64,
}

impl<'a> TradeRow<'a> {
    fn new(market: &'a str, trade: &'a Trade, stream_seq: u64) -> Result<Self> {
        Ok(Self {
            market,
            venue_trade_id: trade.venue_trade_id,
            engine_time_us: trade.engine_time.as_micros(),
            price: decimal::to_clickhouse(trade.price)?,
            size: decimal::to_clickhouse(trade.size)?,
            taker_side: match trade.taker_side {
                TakerSide::Buy => 1,
                TakerSide::Sell => 2,
            },
            taker_order_id: trade.taker_order_id.as_deref(),
            stream_seq,
        })
    }
}

#[derive(Row, Serialize)]
struct GapEventRow<'a> {
    market: String,
    gap_id: &'a str,
    kind: i8,
    time_us: i64,
    reason: i8,
    stream_seq: u64,
}

impl<'a> GapEventRow<'a> {
    fn new(event: &'a GapEvent, stream_seq: u64) -> Self {
        match event {
            GapEvent::Opened {
                market,
                gap_id,
                start,
                reason,
            } => Self {
                market: market.to_string(),
                gap_id,
                kind: 1,
                time_us: start.as_micros(),
                reason: reason_code(*reason),
                stream_seq,
            },
            GapEvent::Closed {
                market,
                gap_id,
                end,
            } => Self {
                market: market.to_string(),
                gap_id,
                kind: 2,
                time_us: end.as_micros(),
                reason: 0,
                stream_seq,
            },
        }
    }
}

fn reason_code(reason: GapReason) -> i8 {
    match reason {
        GapReason::Disconnect => 1,
        GapReason::SequenceBreak => 2,
        GapReason::BookResync => 3,
        GapReason::RecorderDowntime => 4,
        GapReason::StorageOutage => 5,
    }
}

fn reason_from_code(code: i8) -> Result<GapReason> {
    Ok(match code {
        1 => GapReason::Disconnect,
        2 => GapReason::SequenceBreak,
        3 => GapReason::BookResync,
        4 => GapReason::RecorderDowntime,
        5 => GapReason::StorageOutage,
        other => return Err(StoreError::Corrupt(format!("gap reason code {other}"))),
    })
}

#[derive(Row, Deserialize)]
struct FootprintCell {
    bar: i64,
    price: i128,
    buy: i128,
    sell: i128,
    trades: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGap {
    pub id: String,
    pub gap: Gap,
}

#[derive(Row, Deserialize)]
struct GapRow {
    gap_id: String,
    start_us: i64,
    end_us: i64,
    closed: bool,
    reason: i8,
}

impl GapRow {
    fn into_gap(self) -> Result<StoredGap> {
        let start = Timestamp::from_micros(self.start_us);
        let reason = reason_from_code(self.reason)?;
        let gap = if self.closed {
            Gap::closed(start, Timestamp::from_micros(self.end_us), reason).ok_or_else(|| {
                StoreError::Corrupt(format!("gap {} closes before it opens", self.gap_id))
            })?
        } else {
            Gap::open(start, reason)
        };
        Ok(StoredGap {
            id: self.gap_id,
            gap,
        })
    }
}
