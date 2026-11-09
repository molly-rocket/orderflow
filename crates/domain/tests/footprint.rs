mod common;

use common::{buy, d, sell};
use orderflow_domain::{
    Decimal, FootprintBar, FootprintError, PriceGrouping, TickSize, Timeframe, Timestamp,
    footprint_bars,
};

const MINUTE_START: i64 = 1_731_110_460_000_000;

fn grouping(tick: &str, multiple: u32) -> PriceGrouping {
    PriceGrouping::new(TickSize::new(d(tick)).unwrap(), multiple).unwrap()
}

fn minute_bar(grouping: PriceGrouping) -> FootprintBar {
    let range = Timeframe::from_secs(60)
        .unwrap()
        .bar_range(Timestamp::from_micros(MINUTE_START));
    FootprintBar::new(range, grouping)
}

fn rows(bar: &FootprintBar) -> Vec<(Decimal, Decimal, Decimal)> {
    bar.rows().map(|r| (r.low, r.buy, r.sell)).collect()
}

#[test]
fn an_empty_bar_has_no_rows_and_zero_totals() {
    let bar = minute_bar(grouping("0.1", 5));

    assert!(bar.is_empty());
    assert_eq!(rows(&bar), vec![]);
    assert_eq!(bar.total_volume(), d("0"));
    assert_eq!(bar.delta(), d("0"));
    assert_eq!(bar.row_span(), None);
}

#[test]
fn taker_volume_is_bucketed_into_rows_by_price_grouping() {
    let mut bar = minute_bar(grouping("0.1", 5));
    let t = MINUTE_START;
    for trade in [
        buy(t, "100.0", "1.5"),
        buy(t + 1, "100.4", "0.5"),   
        sell(t + 2, "100.2", "3"),    
        sell(t + 3, "100.5", "0.25"),
        buy(t + 4, "99.9", "2"),
    ] {
        bar.add(&trade).unwrap();
    }

    assert_eq!(
        rows(&bar),
        vec![
            (d("99.5"), d("2"), d("0")),
            (d("100.0"), d("2.0"), d("3")),
            (d("100.5"), d("0"), d("0.25")),
        ]
    );
    assert_eq!(bar.buy_volume(), d("4.0"));
    assert_eq!(bar.sell_volume(), d("3.25"));
    assert_eq!(bar.delta(), d("0.75"));
    assert_eq!(bar.total_volume(), d("7.25"));
    assert_eq!(bar.trade_count(), 5);
}

#[test]
fn a_single_row_bar_reports_that_row_as_its_span() {
    let mut bar = minute_bar(grouping("1", 1));
    bar.add(&sell(MINUTE_START, "50", "1")).unwrap();

    let (low, high) = bar.row_span().unwrap();
    assert_eq!(low, high);
    assert_eq!(rows(&bar), vec![(d("50"), d("0"), d("1"))]);
}

#[test]
fn trades_outside_the_bar_are_refused() {
    let mut bar = minute_bar(grouping("1", 1));

    assert_eq!(
        bar.add(&buy(MINUTE_START - 1, "50", "1")),
        Err(FootprintError::TradeOutsideBar)
    );
    assert_eq!(
        bar.add(&buy(MINUTE_START + 60_000_000, "50", "1")),
        Err(FootprintError::TradeOutsideBar)
    );
    assert!(bar.is_empty());
}

#[test]
fn trades_are_split_into_consecutive_bars_by_timeframe() {
    let minute = Timeframe::from_secs(60).unwrap();
    let t = MINUTE_START;
    let trades = [
        buy(t, "100", "1"),
        sell(t + 59_999_999, "100", "2"),
        buy(t + 60_000_000, "101", "4"),
        sell(t + 180_000_000, "99", "8"),
    ];

    let bars = footprint_bars(&trades, minute, grouping("1", 1));

    let summary: Vec<_> = bars
        .iter()
        .map(|b| (b.range().start().as_micros(), b.total_volume()))
        .collect();
    assert_eq!(
        summary,
        vec![
            (t, d("3")),
            (t + 60_000_000, d("4")),
            (t + 180_000_000, d("8")),
        ]
    );
}

#[test]
fn a_bar_can_be_regrouped_to_a_coarser_price_grouping() {
    let mut fine = minute_bar(grouping("0.1", 1));
    let t = MINUTE_START;
    for trade in [
        buy(t, "100.1", "1"),
        sell(t, "100.4", "2"),
        buy(t, "100.5", "4"),
        sell(t, "99.9", "8"),
    ] {
        fine.add(&trade).unwrap();
    }

    let coarse = fine.regroup(grouping("0.1", 5)).unwrap();

    assert_eq!(
        rows(&coarse),
        vec![
            (d("99.5"), d("0"), d("8")),
            (d("100.0"), d("1"), d("2")),
            (d("100.5"), d("4"), d("0")),
        ]
    );
    assert_eq!(coarse.trade_count(), 4);
}

#[test]
fn regrouping_to_a_grouping_that_does_not_nest_is_refused() {
    let fine = minute_bar(grouping("0.1", 2));

    assert_eq!(
        fine.regroup(grouping("0.1", 5)).unwrap_err(),
        FootprintError::GroupingDoesNotNest
    );
    assert_eq!(
        fine.regroup(grouping("0.5", 2)).unwrap_err(),
        FootprintError::GroupingDoesNotNest
    );
}
