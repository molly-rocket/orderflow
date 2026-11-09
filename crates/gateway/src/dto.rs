use std::collections::HashMap;

use orderflow_domain::{
    Decimal, FootprintBar, Gap, ImbalanceSettings, PriceGrouping, PriceRow, TakerSide,
    find_imbalances,
};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MarketDto {
    pub id: String,
    pub venue: &'static str,
    pub market_type: &'static str,
    pub native_name: String,
    pub tick_size: String,
    pub recording_horizon_us: Option<i64>,
    pub burst_grouped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GroupingDto {
    pub tick: String,
    pub multiple: u32,
    pub step: String,
}

impl From<PriceGrouping> for GroupingDto {
    fn from(g: PriceGrouping) -> Self {
        Self {
            tick: g.tick().get().to_string(),
            multiple: g.multiple(),
            step: g.step().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImbalanceDto {
    pub ratio: String,
    pub min_stack: u8,
}

impl From<ImbalanceSettings> for ImbalanceDto {
    fn from(s: ImbalanceSettings) -> Self {
        Self {
            ratio: s.ratio().normalize().to_string(),
            min_stack: s.min_stack(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImbalanceFlag {
    Buy,
    Sell,
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RowDto {
    pub price: String,
    pub buy: String,
    pub sell: String,
    pub imbalance: Option<ImbalanceFlag>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StackDto {
    pub side: &'static str,
    pub low: String,
    pub high: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BarDto {
    pub start_us: i64,
    pub end_us: i64,
    pub incomplete: bool,
    pub buy: String,
    pub sell: String,
    pub delta: String,
    pub total: String,
    pub trades: u64,
    pub rows: Vec<RowDto>,
    pub stacks: Vec<StackDto>,
}

impl BarDto {
    pub fn new(bar: &FootprintBar, settings: &ImbalanceSettings, gaps: &[Gap]) -> Self {
        let found = find_imbalances(bar, settings);
        let mut flags: HashMap<PriceRow, ImbalanceFlag> = HashMap::new();
        for imbalance in &found.imbalances {
            let flag = match imbalance.side {
                TakerSide::Buy => ImbalanceFlag::Buy,
                TakerSide::Sell => ImbalanceFlag::Sell,
            };
            flags
                .entry(imbalance.row)
                .and_modify(|f| *f = ImbalanceFlag::Both)
                .or_insert(flag);
        }
        let grouping = bar.grouping();
        Self {
            start_us: bar.range().start().as_micros(),
            end_us: bar.range().end().as_micros(),
            incomplete: Gap::any_overlaps(gaps, bar.range()),
            buy: decimal(bar.buy_volume()),
            sell: decimal(bar.sell_volume()),
            delta: decimal(bar.delta()),
            total: decimal(bar.total_volume()),
            trades: bar.trade_count(),
            rows: bar
                .rows()
                .map(|r| RowDto {
                    price: decimal(r.low),
                    buy: decimal(r.buy),
                    sell: decimal(r.sell),
                    imbalance: flags.get(&r.row).copied(),
                })
                .collect(),
            stacks: found
                .stacks
                .iter()
                .map(|s| StackDto {
                    side: s.side.as_str(),
                    low: decimal(grouping.row_low(s.low)),
                    high: decimal(grouping.row_low(s.high)),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GapDto {
    pub start_us: i64,
    pub end_us: Option<i64>,
    pub reason: &'static str,
}

impl From<&Gap> for GapDto {
    fn from(gap: &Gap) -> Self {
        Self {
            start_us: gap.start().as_micros(),
            end_us: gap.end().map(orderflow_domain::Timestamp::as_micros),
            reason: gap.reason().as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FootprintResponse {
    pub market: String,
    pub timeframe_secs: u32,
    pub grouping: GroupingDto,
    pub imbalance: ImbalanceDto,
    pub bars: Vec<BarDto>,
    pub gaps: Vec<GapDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LiveMessage {
    Bar { bar: BarDto, closed: bool },
    Gap { gap: GapDto },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorDto {
    pub error: String,
}

fn decimal(value: Decimal) -> String {
    value.normalize().to_string()
}
