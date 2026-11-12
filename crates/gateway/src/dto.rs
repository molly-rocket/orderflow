use std::collections::HashMap;

use orderflow_domain::{
    Bubble, CvdPoint, Decimal, FootprintBar, Gap, ImbalanceSettings, PriceGrouping, PriceRow,
    TakerSide, find_imbalances,
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
    pub open: Option<String>,
    pub high: Option<String>,
    pub low: Option<String>,
    pub close: Option<String>,
    pub cvd: String,
    pub cvd_complete: bool,
    pub rows: Vec<RowDto>,
    pub stacks: Vec<StackDto>,
}

impl BarDto {
    pub fn new(
        bar: &FootprintBar,
        settings: &ImbalanceSettings,
        gaps: &[Gap],
        cvd: CvdPoint,
    ) -> Self {
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
        let ohlc = bar.ohlc();
        Self {
            start_us: bar.range().start().as_micros(),
            end_us: bar.range().end().as_micros(),
            incomplete: Gap::any_overlaps(gaps, bar.range()),
            buy: decimal(bar.buy_volume()),
            sell: decimal(bar.sell_volume()),
            delta: decimal(bar.delta()),
            total: decimal(bar.total_volume()),
            trades: bar.trade_count(),
            open: ohlc.map(|o| decimal(o.open)),
            high: ohlc.map(|o| decimal(o.high)),
            low: ohlc.map(|o| decimal(o.low)),
            close: ohlc.map(|o| decimal(o.close)),
            cvd: decimal(cvd.value),
            cvd_complete: cvd.complete,
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
pub struct BubbleDto {
    pub start_us: i64,
    pub end_us: i64,
    pub side: &'static str,
    pub vwap: String,
    pub size: String,
    pub notional: String,
    pub trades: u32,
    pub low: String,
    pub high: String,
    pub first_trade_id: u64,
    pub large: bool,
}

impl BubbleDto {
    pub fn new(bubble: &Bubble, large: bool) -> Self {
        Self {
            start_us: bubble.start.as_micros(),
            end_us: bubble.end.as_micros(),
            side: bubble.side.as_str(),
            vwap: decimal(bubble.vwap().round_dp(VWAP_DECIMALS)),
            size: decimal(bubble.size),
            notional: decimal(bubble.notional),
            trades: bubble.trade_count,
            low: decimal(bubble.low),
            high: decimal(bubble.high),
            first_trade_id: bubble.first_trade_id,
            large,
        }
    }
}

const VWAP_DECIMALS: u32 = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LargeBubbleDto {
    pub percentile: String,
    pub usd_floor: Option<String>,
    pub threshold: Option<String>,
    pub window_start_us: i64,
    pub window_end_us: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BubblesResponse {
    pub market: String,
    pub burst_grouped: bool,
    pub large: LargeBubbleDto,
    pub bubbles: Vec<BubbleDto>,
    pub truncated: bool,
    pub gaps: Vec<GapDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LiveMessage {
    Bar { bar: BarDto, closed: bool },
    Gap { gap: GapDto },
    Bubble { bubble: BubbleDto },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LadderRowDto {
    pub price: String,
    pub bid: String,
    pub ask: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BookDto {
    pub time_us: i64,
    pub status: &'static str,
    pub best_bid: Option<String>,
    pub best_ask: Option<String>,
    pub known_low: Option<String>,
    pub known_high: Option<String>,
    pub step: String,
    pub rows: Vec<LadderRowDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HeatmapFrameDto {
    pub time_us: i64,
    pub status: &'static str,
    pub step: String,
    pub low: Option<String>,
    pub sizes: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BookMessage {
    Book { book: BookDto },
    HeatmapHistory { frames: Vec<HeatmapFrameDto> },
    Heatmap { frame: HeatmapFrameDto },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorDto {
    pub error: String,
}

pub(crate) fn decimal(value: Decimal) -> String {
    value.normalize().to_string()
}
