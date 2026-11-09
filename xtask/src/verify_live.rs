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
struct BarDto {
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

#[derive(Debug, Deserialize)]
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
                let trades = binance_trades(&market.native_name, start / 1000, end / 1000)?;
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
            }
            start = end;
        }

        self.verify_completeness(market, last_start + MINUTE_US, report)
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
