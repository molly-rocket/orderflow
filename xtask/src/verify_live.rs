use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail, ensure};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::DeserializeOwned;

const BINANCE_FAPI: &str = "https://fapi.binance.com";
const USER_AGENT: &str = "orderflow-xtask-verify-live";
const PAGE: usize = 1000;
const GROUPINGS: [u32; 3] = [1, 5, 50];
const IMBALANCE_RATIO: u32 = 3;
const MIN_STACK: usize = 3;
const SETTLE_SECS: i64 = 60;
const MAX_WINDOW_SECS: i64 = 23 * 3600;
const MINUTE_US: i64 = 60_000_000;
const DAY_US: i64 = 86_400_000_000;
const BUBBLE_WINDOW_MS: i64 = 100;
const BUBBLE_LOOKBACK_MS: i64 = 1_000;
const VWAP_DECIMALS: u32 = 8;
const BOOK_TOUCH_TOLERANCE: &str = "0.0005";
const BOOK_RANGE_TOLERANCE: &str = "0.003";
const BOOK_TOUCH_LEVELS: usize = 20;
const BOOK_MIN_PRESENCE: &str = "0.85";
const BOOK_TOTAL_RATIO: (&str, &str) = ("0.5", "2");
const BOOK_ATTEMPTS: usize = 3;
const BOOK_WAIT_SECS: u64 = 15;

#[derive(Debug)]
pub(crate) struct Options {
    gateway: String,
    minutes: u32,
}

impl Options {
    pub(crate) fn parse(args: &[String]) -> Result<Self> {
        let mut options = Self {
            gateway: "http://127.0.0.1:8080".into(),
            minutes: 15,
        };
        let mut args = args.iter();
        while let Some(flag) = args.next() {
            let mut value = || {
                args.next()
                    .ok_or_else(|| anyhow!("{flag} needs a value"))
                    .cloned()
            };
            match flag.as_str() {
                "--gateway" => value()?
                    .trim_end_matches('/')
                    .clone_into(&mut options.gateway),
                "--minutes" => options.minutes = value()?.parse().context("--minutes")?,
                other => bail!("unknown flag {other}; expected --gateway URL, --minutes N"),
            }
        }
        ensure!(options.minutes > 0, "--minutes must be at least 1");
        Ok(options)
    }
}

pub(crate) fn run(options: &Options) -> Result<()> {
    let checker = Checker {
        gateway: &options.gateway,
    };
    let markets: Vec<_> = checker
        .markets()?
        .into_iter()
        .filter(|m| m.venue == "binance" && m.market_type == "perp")
        .collect();
    ensure!(
        !markets.is_empty(),
        "{} serves no Binance perp Market",
        options.gateway
    );

    let mut report = Report::default();
    for market in &markets {
        checker.verify_market(market, options.minutes, &mut report)?;
    }
    println!(
        "{} checks, {} mismatches, {} skipped",
        report.checks, report.failures, report.skipped
    );
    ensure!(
        report.failures == 0,
        "the gateway disagrees with Binance ({} mismatches)",
        report.failures
    );
    ensure!(report.checks > 0, "nothing was checked");
    Ok(())
}

#[derive(Debug, Default)]
struct Report {
    checks: usize,
    failures: usize,
    skipped: usize,
}

impl Report {
    fn record(&mut self, label: &str, mismatches: &[String]) {
        self.checks += 1;
        if mismatches.is_empty() {
            println!("{label} IDENTICAL");
        } else {
            self.failures += 1;
            println!("{label} MISMATCH");
            for mismatch in mismatches.iter().take(8) {
                println!("    {mismatch}");
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct MarketDto {
    id: String,
    venue: String,
    market_type: String,
    native_name: String,
    tick_size: String,
    recording_horizon_us: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct BookRowDto {
    price: String,
    bid: String,
    ask: String,
}

#[derive(Debug, Deserialize)]
struct BookDto {
    status: String,
    best_bid: Option<String>,
    best_ask: Option<String>,
    known_low: Option<String>,
    known_high: Option<String>,
    rows: Vec<BookRowDto>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BookMessageDto {
    Book { book: BookDto },
    HeatmapHistory {},
    Heatmap {},
}

#[derive(Debug, Deserialize)]
struct DepthDto {
    bids: Vec<[String; 2]>,
    asks: Vec<[String; 2]>,
}

#[derive(Debug, Deserialize)]
struct GapDto {
    start_us: i64,
    end_us: Option<i64>,
    reason: String,
}

#[derive(Debug, Deserialize)]
struct FootprintDto {
    bars: Vec<BarDto>,
    gaps: Vec<GapDto>,
}

#[derive(Debug, Deserialize)]
struct BubblesDto {
    bubbles: Vec<ServedBubble>,
    truncated: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct ServedBubble {
    start_us: i64,
    end_us: i64,
    side: String,
    vwap: String,
    size: String,
    notional: String,
    trades: u32,
    low: String,
    high: String,
    first_trade_id: u64,
}

#[derive(Debug, Deserialize)]
struct BarDto {
    start_us: i64,
    cvd: String,
    incomplete: bool,
    buy: String,
    sell: String,
    delta: String,
    total: String,
    trades: u64,
    rows: Vec<RowDto>,
    stacks: Vec<StackDto>,
}

#[derive(Debug, Deserialize)]
struct RowDto {
    price: String,
    buy: String,
    sell: String,
    imbalance: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StackDto {
    side: String,
    low: String,
    high: String,
}

#[derive(Debug, Clone, Deserialize)]
struct AggTrade {
    a: u64,
    p: String,
    q: String,
    #[serde(rename = "T")]
    time_ms: i64,
    m: bool,
}

#[derive(Debug, Deserialize)]
struct ExchangeInfo {
    symbols: Vec<SymbolInfo>,
}

#[derive(Debug, Deserialize)]
struct SymbolInfo {
    symbol: String,
    filters: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Side {
    Buy,
    Sell,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Row {
    buy: Decimal,
    sell: Decimal,
    imbalances: BTreeSet<Side>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Bar {
    trades: u64,
    rows: BTreeMap<i64, Row>,
    stacks: BTreeSet<(Side, i64, i64)>,
}

impl Bar {
    fn from_trades(trades: &[AggTrade], step: Decimal) -> Result<Self> {
        let mut bar = Self {
            trades: trades.len() as u64,
            ..Self::default()
        };
        for trade in trades {
            let row = row_index(dec(&trade.p)?, step)?;
            let quantity = dec(&trade.q)?;
            let cell = bar.rows.entry(row).or_default();
            if trade.m {
                cell.sell += quantity;
            } else {
                cell.buy += quantity;
            }
        }
        bar.flag_imbalances();
        Ok(bar)
    }

    fn flag_imbalances(&mut self) {
        let (Some(&low), Some(&high)) = (self.rows.keys().next(), self.rows.keys().next_back())
        else {
            return;
        };
        let ratio = Decimal::from(IMBALANCE_RATIO);
        let volume = |rows: &BTreeMap<i64, Row>, row: i64, side: Side| {
            rows.get(&row).map_or(Decimal::ZERO, |r| match side {
                Side::Buy => r.buy,
                Side::Sell => r.sell,
            })
        };
        let mut flags = Vec::new();
        for (&row, cell) in &self.rows {
            if cell.buy > Decimal::ZERO
                && row > low
                && cell.buy >= ratio * volume(&self.rows, row - 1, Side::Sell)
            {
                flags.push((row, Side::Buy));
            }
            if cell.sell > Decimal::ZERO
                && row < high
                && cell.sell >= ratio * volume(&self.rows, row + 1, Side::Buy)
            {
                flags.push((row, Side::Sell));
            }
        }
        for (row, side) in flags {
            if let Some(cell) = self.rows.get_mut(&row) {
                cell.imbalances.insert(side);
            }
        }
        for side in [Side::Buy, Side::Sell] {
            let mut run: Option<(i64, i64)> = None;
            for row in low..=high + 1 {
                let flagged = self
                    .rows
                    .get(&row)
                    .is_some_and(|r| r.imbalances.contains(&side));
                run = match (run, flagged) {
                    (Some((start, _)), true) => Some((start, row)),
                    (None, true) => Some((row, row)),
                    (Some((start, end)), false) => {
                        if usize::try_from(end - start + 1).unwrap_or(0) >= MIN_STACK {
                            self.stacks.insert((side, start, end));
                        }
                        None
                    }
                    (None, false) => None,
                };
            }
        }
    }

    fn buy(&self) -> Decimal {
        self.rows.values().map(|r| r.buy).sum()
    }

    fn sell(&self) -> Decimal {
        self.rows.values().map(|r| r.sell).sum()
    }
}

fn served_bar(dto: &BarDto, step: Decimal, mismatches: &mut Vec<String>) -> Result<Bar> {
    let mut bar = Bar {
        trades: dto.trades,
        ..Bar::default()
    };
    for row in &dto.rows {
        let imbalances = match row.imbalance.as_deref() {
            None => BTreeSet::new(),
            Some("buy") => BTreeSet::from([Side::Buy]),
            Some("sell") => BTreeSet::from([Side::Sell]),
            Some("both") => BTreeSet::from([Side::Buy, Side::Sell]),
            Some(other) => bail!("unknown imbalance flag {other:?}"),
        };
        let index = row_index(dec(&row.price)?, step)?;
        if dec(&row.price)? != Decimal::from(index) * step {
            mismatches.push(format!("row price {} is not on the {step} grid", row.price));
        }
        bar.rows.insert(
            index,
            Row {
                buy: dec(&row.buy)?,
                sell: dec(&row.sell)?,
                imbalances,
            },
        );
    }
    for stack in &dto.stacks {
        let side = match stack.side.as_str() {
            "buy" => Side::Buy,
            "sell" => Side::Sell,
            other => bail!("unknown stack side {other:?}"),
        };
        bar.stacks.insert((
            side,
            row_index(dec(&stack.low)?, step)?,
            row_index(dec(&stack.high)?, step)?,
        ));
    }
    let (buy, sell) = (dec(&dto.buy)?, dec(&dto.sell)?);
    if buy != bar.buy() || sell != bar.sell() {
        mismatches.push(format!(
            "bar totals {buy}/{sell} differ from its own rows {}/{}",
            bar.buy(),
            bar.sell()
        ));
    }
    if dec(&dto.delta)? != buy - sell || dec(&dto.total)? != buy + sell {
        mismatches.push(format!(
            "delta {} / total {} inconsistent with buy {buy} sell {sell}",
            dto.delta, dto.total
        ));
    }
    if dto.incomplete {
        mismatches.push("bar is marked incomplete outside any Gap".into());
    }
    Ok(bar)
}

fn compare(expected: &Bar, served: &Bar, step: Decimal, mismatches: &mut Vec<String>) {
    if served.trades != expected.trades {
        mismatches.push(format!(
            "trades {} != Binance {}",
            served.trades, expected.trades
        ));
    }
    let price = |row: i64| Decimal::from(row) * step;
    let rows: BTreeSet<i64> = expected
        .rows
        .keys()
        .chain(served.rows.keys())
        .copied()
        .collect();
    for row in rows {
        let (want, got) = (expected.rows.get(&row), served.rows.get(&row));
        if want != got {
            mismatches.push(format!(
                "row {}: served {got:?}, Binance {want:?}",
                price(row)
            ));
        }
    }
    for stack in expected.stacks.symmetric_difference(&served.stacks) {
        let source = if expected.stacks.contains(stack) {
            "missing"
        } else {
            "extra"
        };
        mismatches.push(format!(
            "{source} {:?} stack {}..{}",
            stack.0,
            price(stack.1),
            price(stack.2)
        ));
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ExpectedBubble {
    buy: bool,
    start_ms: i64,
    end_ms: i64,
    first_id: u64,
    last_id: u64,
    trades: u32,
    size: Decimal,
    notional: Decimal,
    low: Decimal,
    high: Decimal,
}

impl ExpectedBubble {
    fn starting(trade: &AggTrade) -> Result<Self> {
        let (price, quantity) = (dec(&trade.p)?, dec(&trade.q)?);
        Ok(Self {
            buy: !trade.m,
            start_ms: trade.time_ms,
            end_ms: trade.time_ms,
            first_id: trade.a,
            last_id: trade.a,
            trades: 1,
            size: quantity,
            notional: price * quantity,
            low: price,
            high: price,
        })
    }

    fn takes(&self, trade: &AggTrade) -> bool {
        self.buy != trade.m
            && trade.a == self.last_id + 1
            && trade.time_ms - self.start_ms < BUBBLE_WINDOW_MS
    }

    fn add(&mut self, trade: &AggTrade) -> Result<()> {
        let (price, quantity) = (dec(&trade.p)?, dec(&trade.q)?);
        self.end_ms = trade.time_ms;
        self.last_id = trade.a;
        self.trades += 1;
        self.size += quantity;
        self.notional += price * quantity;
        self.low = self.low.min(price);
        self.high = self.high.max(price);
        Ok(())
    }

    fn matches(&self, served: &ServedBubble) -> Result<bool> {
        let vwap = (self.notional / self.size).round_dp(VWAP_DECIMALS);
        Ok(served.side == if self.buy { "buy" } else { "sell" }
            && served.start_us == self.start_ms * 1000
            && served.end_us == self.end_ms * 1000
            && served.first_trade_id == self.first_id
            && served.trades == self.trades
            && dec(&served.size)? == self.size
            && dec(&served.notional)? == self.notional
            && dec(&served.vwap)? == vwap
            && dec(&served.low)? == self.low
            && dec(&served.high)? == self.high)
    }
}

fn expected_bubbles(trades: &[AggTrade]) -> Result<Vec<ExpectedBubble>> {
    let mut found: Vec<ExpectedBubble> = Vec::new();
    for trade in trades {
        match found.last_mut() {
            Some(open) if open.takes(trade) => open.add(trade)?,
            _ => found.push(ExpectedBubble::starting(trade)?),
        }
    }
    Ok(found)
}

fn taker_delta(trades: &[AggTrade]) -> Result<Decimal> {
    trades.iter().try_fold(Decimal::ZERO, |sum, t| {
        let quantity = dec(&t.q)?;
        Ok(if t.m { sum - quantity } else { sum + quantity })
    })
}

fn row_index(price: Decimal, step: Decimal) -> Result<i64> {
    i64::try_from((price / step).floor()).context("row index out of range")
}

fn dec(s: &str) -> Result<Decimal> {
    s.parse().with_context(|| format!("not a decimal: {s:?}"))
}

#[derive(Debug)]
struct Checker<'a> {
    gateway: &'a str,
}

impl Checker<'_> {
    fn markets(&self) -> Result<Vec<MarketDto>> {
        get_json(&format!("{}/api/v1/markets", self.gateway))
    }

    fn gaps(&self, market: &str, from_us: i64, to_us: i64) -> Result<Vec<GapDto>> {
        get_json(&format!(
            "{}/api/v1/gaps?market={market}&from_us={from_us}&to_us={to_us}",
            self.gateway
        ))
    }

    fn footprint(
        &self,
        market: &str,
        from_us: i64,
        to_us: i64,
        grouping: u32,
    ) -> Result<FootprintDto> {
        get_json(&format!(
            "{}/api/v1/footprint?market={market}&from_us={from_us}&to_us={to_us}\
             &timeframe_secs=60&grouping={grouping}\
             &imbalance_ratio={IMBALANCE_RATIO}&min_stack={MIN_STACK}",
            self.gateway
        ))
    }

    fn bubbles(&self, market: &str, from_us: i64, to_us: i64) -> Result<BubblesDto> {
        get_json(&format!(
            "{}/api/v1/bubbles?market={market}&from_us={from_us}&to_us={to_us}&min_notional=0",
            self.gateway
        ))
    }

    fn verify_bubbles(
        &self,
        market: &MarketDto,
        start: i64,
        around: &[AggTrade],
        report: &mut Report,
    ) -> Result<()> {
        let end = start + MINUTE_US;
        let expected: Vec<ExpectedBubble> = expected_bubbles(around)?
            .into_iter()
            .filter(|b| b.start_ms * 1000 >= start && b.start_ms * 1000 < end)
            .collect();
        let served = self.bubbles(&market.id, start, end)?;
        let mut mismatches = Vec::new();
        if served.truncated {
            mismatches.push("Bubbles truncated".to_owned());
        }
        if served.bubbles.len() != expected.len() {
            mismatches.push(format!(
                "{} Bubbles served, Binance gives {}",
                served.bubbles.len(),
                expected.len()
            ));
        }
        for (want, got) in expected.iter().zip(&served.bubbles) {
            if !want.matches(got)? {
                mismatches.push(format!("served {got:?}, Binance {want:?}"));
            }
        }
        let large_trades: u32 = expected.iter().map(|b| b.trades).max().unwrap_or(0);
        report.record(
            &format!(
                "{} {start} bubbles={:4} largest_run={large_trades:3}",
                market.id,
                expected.len()
            ),
            &mismatches,
        );
        Ok(())
    }

    fn verify_cvd_step(
        &self,
        market: &MarketDto,
        start: i64,
        minute_trades: &[AggTrade],
        report: &mut Report,
    ) -> Result<()> {
        if start % DAY_US == 0 {
            return Ok(());
        }
        let served = self.footprint(&market.id, start - MINUTE_US, start + MINUTE_US, 50)?;
        let (Some(previous), Some(current)) = (
            served.bars.iter().find(|b| b.start_us == start - MINUTE_US),
            served.bars.iter().find(|b| b.start_us == start),
        ) else {
            return Ok(());
        };
        let step = dec(&current.cvd)? - dec(&previous.cvd)?;
        let binance = taker_delta(minute_trades)?;
        let mismatches = if step == binance {
            Vec::new()
        } else {
            vec![format!("CVD moved {step}, Binance Taker delta {binance}")]
        };
        report.record(
            &format!("{} {start} cvd step={step}", market.id),
            &mismatches,
        );
        Ok(())
    }

    fn verify_cvd_today(&self, market: &MarketDto, end_us: i64, report: &mut Report) -> Result<()> {
        let day_start = end_us.div_euclid(DAY_US) * DAY_US;
        let gaps = self.gaps(&market.id, day_start, end_us)?;
        let horizon = market.recording_horizon_us.unwrap_or(i64::MAX);
        if horizon > day_start || !gaps.is_empty() || end_us == day_start {
            report.skipped += 1;
            println!(
                "{} cvd since 00:00 UTC skipped: {}",
                market.id,
                if gaps.is_empty() {
                    "recording began after 00:00 UTC"
                } else {
                    "a Gap today"
                }
            );
            return Ok(());
        }
        let served = self.footprint(&market.id, end_us - MINUTE_US, end_us, 50)?;
        let Some(last) = served.bars.last() else {
            return Ok(());
        };
        let klines: Vec<Vec<serde_json::Value>> = get_json(&format!(
            "{BINANCE_FAPI}/fapi/v1/klines?symbol={}&interval=1m&startTime={}&endTime={}&limit=1500",
            market.native_name,
            day_start / 1000,
            end_us / 1000 - 1
        ))?;
        let mut binance = Decimal::ZERO;
        for kline in &klines {
            let field = |i: usize| -> Result<Decimal> {
                dec(kline
                    .get(i)
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("kline field {i}"))?)
            };
            binance += field(9)? * Decimal::TWO - field(5)?;
        }
        let served_cvd = dec(&last.cvd)?;
        let mismatches = if served_cvd == binance {
            Vec::new()
        } else {
            vec![format!("CVD {served_cvd}, Binance klines {binance}")]
        };
        report.record(
            &format!(
                "{} cvd since 00:00 UTC = {served_cvd} ({} minutes)",
                market.id,
                klines.len()
            ),
            &mismatches,
        );
        Ok(())
    }

    fn verify_market(&self, market: &MarketDto, minutes: u32, report: &mut Report) -> Result<()> {
        let tick = dec(&market.tick_size)?;
        let binance_tick = binance_tick_size(&market.native_name)?;
        let mut mismatches = Vec::new();
        if tick != binance_tick {
            mismatches.push(format!(
                "gateway tick {tick} != Binance tick {binance_tick}"
            ));
        }
        report.record(&format!("{} tick size {tick}", market.id), &mismatches);

        let now_us = i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros())?;
        let last_start = (now_us - SETTLE_SECS * 1_000_000) / MINUTE_US * MINUTE_US - MINUTE_US;
        let first_start = last_start - i64::from(minutes - 1) * MINUTE_US;
        let gaps = self.gaps(&market.id, first_start, last_start + MINUTE_US)?;
        let horizon = market.recording_horizon_us.unwrap_or(i64::MAX);

        let mut start = first_start;
        while start <= last_start {
            let end = start + MINUTE_US;
            if start < horizon {
                report.skipped += 1;
                println!(
                    "{} {start} skipped: before the Recording Horizon",
                    market.id
                );
            } else if let Some(gap) = gaps
                .iter()
                .find(|g| g.start_us < end && g.end_us.is_none_or(|e| e > start))
            {
                report.skipped += 1;
                println!(
                    "{} {start} skipped: overlaps a {} Gap",
                    market.id, gap.reason
                );
            } else {
                let around = binance_trades(
                    &market.native_name,
                    start / 1000 - BUBBLE_LOOKBACK_MS,
                    end / 1000 + BUBBLE_LOOKBACK_MS,
                )?;
                let trades: Vec<AggTrade> = around
                    .iter()
                    .filter(|t| t.time_ms * 1000 >= start && t.time_ms * 1000 < end)
                    .cloned()
                    .collect();
                for grouping in GROUPINGS {
                    let step = tick * Decimal::from(grouping);
                    let expected = Bar::from_trades(&trades, step)?;
                    let served = self.footprint(&market.id, start, end, grouping)?;
                    let mut mismatches = Vec::new();
                    match served.bars.as_slice() {
                        [bar] => {
                            let served = served_bar(bar, step, &mut mismatches)?;
                            compare(&expected, &served, step, &mut mismatches);
                        }
                        [] if trades.is_empty() => {}
                        bars => {
                            mismatches.push(format!("{} bars served for one minute", bars.len()));
                        }
                    }
                    if !served.gaps.is_empty() {
                        mismatches.push(format!("Gaps served: {:?}", served.gaps));
                    }
                    let imbalances: usize =
                        expected.rows.values().map(|r| r.imbalances.len()).sum();
                    report.record(
                        &format!(
                            "{} {start} x{grouping:<2} trades={:5} rows={:4} imbalances={imbalances:4} stacks={:3}",
                            market.id,
                            expected.trades,
                            expected.rows.len(),
                            expected.stacks.len()
                        ),
                        &mismatches,
                    );
                }
                self.verify_bubbles(market, start, &around, report)?;
                self.verify_cvd_step(market, start, &trades, report)?;
            }
            start = end;
        }

        self.verify_cvd_today(market, last_start + MINUTE_US, report)?;
        self.verify_book(market, report);
        self.verify_completeness(market, last_start + MINUTE_US, report)
    }

    fn verify_book(&self, market: &MarketDto, report: &mut Report) {
        let label = format!("{} order book vs Binance depth", market.id);
        let mut session = match BookSession::open(self.gateway, &market.id) {
            Ok(session) => session,
            Err(error) => {
                report.record(&label, &[format!("order book unavailable: {error:#}")]);
                return;
            }
        };
        let mut last = Vec::new();
        for attempt in 1..=BOOK_ATTEMPTS {
            let outcome = binance_depth(&market.native_name)
                .map_err(|e| format!("Binance depth unavailable: {e:#}"))
                .and_then(|depth| {
                    let book = session
                        .next_live()
                        .map_err(|e| format!("order book unavailable: {e:#}"))?;
                    compare_book(&book, &depth).map_err(|e| format!("{e:#}"))
                });
            last = match outcome {
                Ok(mismatches) if mismatches.is_empty() => {
                    report.record(&format!("{label} (attempt {attempt})"), &[]);
                    return;
                }
                Ok(mismatches) => mismatches,
                Err(error) => vec![error],
            };
        }
        report.record(&label, &last);
    }

    fn verify_completeness(
        &self,
        market: &MarketDto,
        end_us: i64,
        report: &mut Report,
    ) -> Result<()> {
        let earliest = end_us - MAX_WINDOW_SECS * 1_000_000;
        let gaps = self.gaps(&market.id, earliest, end_us)?;
        if gaps.iter().any(|g| g.end_us.is_none()) {
            report.skipped += 1;
            println!("{} completeness skipped: a Gap is still open", market.id);
            return Ok(());
        }
        let after_gaps = gaps
            .iter()
            .filter_map(|g| g.end_us)
            .max()
            .unwrap_or(earliest);
        let horizon = market.recording_horizon_us.unwrap_or(i64::MAX);
        let start_us = (after_gaps
            .max(earliest)
            .max(horizon)
            .saturating_add(MINUTE_US - 1))
            / MINUTE_US
            * MINUTE_US;
        if start_us >= end_us {
            report.skipped += 1;
            println!("{} completeness skipped: no Gap-free minute", market.id);
            return Ok(());
        }
        let footprint = self.footprint(&market.id, start_us, end_us, 10_000)?;
        let served: u64 = footprint.bars.iter().map(|b| b.trades).sum();
        let binance = first_trade_id_at(&market.native_name, end_us / 1000)?
            - first_trade_id_at(&market.native_name, start_us / 1000)?;
        let mut mismatches = Vec::new();
        if !footprint.gaps.is_empty() {
            mismatches.push(format!("Gaps served: {:?}", footprint.gaps));
        }
        if served != binance {
            mismatches.push(format!(
                "gateway Trades {served} != Binance id span {binance}"
            ));
        }
        #[allow(clippy::cast_precision_loss, reason = "hours, for display only")]
        let hours = (end_us - start_us) as f64 / 3.6e9;
        report.record(
            &format!(
                "{} completeness {start_us}..{end_us} ({hours:.2} h) trades={served}",
                market.id
            ),
            &mismatches,
        );
        Ok(())
    }
}

fn binance_trades(symbol: &str, start_ms: i64, end_ms: i64) -> Result<Vec<AggTrade>> {
    let mut trades: Vec<AggTrade> = get_json(&format!(
        "{BINANCE_FAPI}/fapi/v1/aggTrades?symbol={symbol}&startTime={start_ms}&endTime={}&limit={PAGE}",
        end_ms - 1
    ))?;
    while let Some(last) = trades.last() {
        let page: Vec<AggTrade> = get_json(&format!(
            "{BINANCE_FAPI}/fapi/v1/aggTrades?symbol={symbol}&fromId={}&limit={PAGE}",
            last.a + 1
        ))?;
        let before = trades.len();
        trades.extend(page.into_iter().take_while(|t| t.time_ms < end_ms));
        if trades.len() - before < PAGE {
            break;
        }
    }
    for pair in trades.windows(2) {
        ensure!(
            pair[1].a == pair[0].a + 1,
            "Binance aggTrade ids jump {} -> {}",
            pair[0].a,
            pair[1].a
        );
    }
    Ok(trades)
}

fn first_trade_id_at(symbol: &str, ms: i64) -> Result<u64> {
    let first: Vec<AggTrade> = get_json(&format!(
        "{BINANCE_FAPI}/fapi/v1/aggTrades?symbol={symbol}&startTime={ms}&endTime={}&limit=1",
        ms + 59_999
    ))?;
    first
        .first()
        .map(|t| t.a)
        .ok_or_else(|| anyhow!("no {symbol} trade within a minute of {ms}"))
}

fn binance_tick_size(symbol: &str) -> Result<Decimal> {
    let info: ExchangeInfo = get_json(&format!("{BINANCE_FAPI}/fapi/v1/exchangeInfo"))?;
    let symbol_info = info
        .symbols
        .iter()
        .find(|s| s.symbol == symbol)
        .ok_or_else(|| anyhow!("Binance does not list {symbol}"))?;
    let tick = symbol_info
        .filters
        .iter()
        .find(|f| f["filterType"] == "PRICE_FILTER")
        .and_then(|f| f["tickSize"].as_str())
        .ok_or_else(|| anyhow!("{symbol} has no PRICE_FILTER tickSize"))?;
    dec(tick)
}

struct BookSession {
    socket: tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>,
}

impl BookSession {
    fn open(gateway: &str, market: &str) -> Result<Self> {
        let url = format!(
            "{}/api/v1/book?market={}&grouping=1",
            gateway.replacen("http", "ws", 1),
            urlencode(market)
        );
        let (mut socket, _) =
            tungstenite::connect(&url).with_context(|| format!("connecting {url}"))?;
        if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_mut() {
            stream.set_read_timeout(Some(std::time::Duration::from_secs(BOOK_WAIT_SECS)))?;
        }
        Ok(Self { socket })
    }

    fn next_live(&mut self) -> Result<BookDto> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(BOOK_WAIT_SECS);
        let mut last_status = String::from("none");
        while std::time::Instant::now() < deadline {
            let message = self.socket.read().context("reading the book socket")?;
            let tungstenite::Message::Text(text) = message else {
                continue;
            };
            let parsed: BookMessageDto =
                serde_json::from_str(&text).context("decoding a book message")?;
            if let BookMessageDto::Book { book } = parsed {
                if book.status == "live" {
                    return Ok(book);
                }
                last_status = book.status;
            }
        }
        bail!("no live book within {BOOK_WAIT_SECS}s (last status {last_status})")
    }
}

fn urlencode(value: &str) -> String {
    value.replace(':', "%3A")
}

fn binance_depth(symbol: &str) -> Result<DepthDto> {
    get_json(&format!(
        "{BINANCE_FAPI}/fapi/v1/depth?symbol={symbol}&limit=1000"
    ))
}

fn within(a: Decimal, b: Decimal, tolerance: Decimal) -> bool {
    (a - b).abs() <= a.abs().max(b.abs()) * tolerance
}

fn compare_book(book: &BookDto, depth: &DepthDto) -> Result<Vec<String>> {
    let touch_tolerance = dec(BOOK_TOUCH_TOLERANCE)?;
    let range_tolerance = dec(BOOK_RANGE_TOLERANCE)?;
    let min_presence = dec(BOOK_MIN_PRESENCE)?;
    let (ratio_low, ratio_high) = (dec(BOOK_TOTAL_RATIO.0)?, dec(BOOK_TOTAL_RATIO.1)?);
    let mut mismatches = Vec::new();
    let level =
        |raw: &[String; 2]| -> Result<(Decimal, Decimal)> { Ok((dec(&raw[0])?, dec(&raw[1])?)) };
    let (Some(bid), Some(ask)) = (book.best_bid.as_deref(), book.best_ask.as_deref()) else {
        mismatches.push("live book without a best bid and ask".into());
        return Ok(mismatches);
    };
    let (bid, ask) = (dec(bid)?, dec(ask)?);
    if bid >= ask {
        mismatches.push(format!("best bid {bid} is not below best ask {ask}"));
    }
    let (Some(top_bid), Some(top_ask)) = (depth.bids.first(), depth.asks.first()) else {
        mismatches.push("Binance depth is empty".into());
        return Ok(mismatches);
    };
    let (binance_bid, _) = level(top_bid)?;
    let (binance_ask, _) = level(top_ask)?;
    if !within(bid, binance_bid, touch_tolerance) {
        mismatches.push(format!("best bid {bid} vs Binance {binance_bid}"));
    }
    if !within(ask, binance_ask, touch_tolerance) {
        mismatches.push(format!("best ask {ask} vs Binance {binance_ask}"));
    }
    let (Some(low), Some(high)) = (book.known_low.as_deref(), book.known_high.as_deref()) else {
        mismatches.push("live book without a known range".into());
        return Ok(mismatches);
    };
    let (low, high) = (dec(low)?, dec(high)?);
    let (deepest_bid, _) = level(depth.bids.last().unwrap_or(top_bid))?;
    let (deepest_ask, _) = level(depth.asks.last().unwrap_or(top_ask))?;
    if !within(low, deepest_bid, range_tolerance) {
        mismatches.push(format!(
            "known low {low} vs Binance deepest bid {deepest_bid}"
        ));
    }
    if !within(high, deepest_ask, range_tolerance) {
        mismatches.push(format!(
            "known high {high} vs Binance deepest ask {deepest_ask}"
        ));
    }
    let mut served_bids: BTreeMap<Decimal, Decimal> = BTreeMap::new();
    let mut served_asks: BTreeMap<Decimal, Decimal> = BTreeMap::new();
    for row in &book.rows {
        let price = dec(&row.price)?;
        let (bid_size, ask_size) = (dec(&row.bid)?, dec(&row.ask)?);
        if !bid_size.is_zero() {
            served_bids.insert(price, bid_size);
        }
        if !ask_size.is_zero() {
            served_asks.insert(price, ask_size);
        }
    }
    for (side, served, binance) in [
        ("bids", &served_bids, &depth.bids),
        ("asks", &served_asks, &depth.asks),
    ] {
        let mut in_range = 0u32;
        let mut present = 0u32;
        let mut binance_total = Decimal::ZERO;
        let mut floor = high;
        let mut ceiling = low;
        for (i, raw) in binance.iter().enumerate() {
            let (price, size) = level(raw)?;
            if price < low || price > high {
                continue;
            }
            floor = floor.min(price);
            ceiling = ceiling.max(price);
            binance_total += size;
            if i < BOOK_TOUCH_LEVELS {
                continue;
            }
            in_range += 1;
            if served.contains_key(&price) {
                present += 1;
            }
        }
        if in_range > 0 {
            let presence = Decimal::from(present) / Decimal::from(in_range);
            if presence < min_presence {
                mismatches.push(format!(
                    "{side}: {present} of {in_range} Binance levels inside the known range (beyond the top {BOOK_TOUCH_LEVELS}) are in the served book"
                ));
            }
        }
        let served_total: Decimal = served.range(floor..=ceiling).map(|(_, size)| *size).sum();
        if !binance_total.is_zero() {
            let ratio = served_total / binance_total;
            if ratio < ratio_low || ratio > ratio_high {
                mismatches.push(format!(
                    "{side}: served total {served_total} vs Binance {binance_total} over {floor}..{ceiling} (ratio {ratio:.2})"
                ));
            }
        }
    }
    Ok(mismatches)
}

fn get_json<T: DeserializeOwned>(url: &str) -> Result<T> {
    let body = ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .with_context(|| format!("GET {url}"))?
        .body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_to_string()
        .with_context(|| format!("reading {url}"))?;
    serde_json::from_str(&body).with_context(|| format!("decoding {url}"))
}
