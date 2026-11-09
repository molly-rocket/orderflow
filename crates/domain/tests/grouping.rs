mod common;

use common::d;
use orderflow_domain::{
    PriceGrouping, PriceGroupingError, TickSize, TimeRange, Timeframe, TimeframeError, Timestamp,
};

#[test]
fn a_price_grouping_is_a_whole_multiple_of_the_tick_size() {
    let tick = TickSize::new(d("0.1")).unwrap();

    assert_eq!(PriceGrouping::new(tick, 5).unwrap().step(), d("0.5"));
    assert_eq!(
        PriceGrouping::from_step(tick, d("2.5")).unwrap().multiple(),
        25
    );
    assert_eq!(
        PriceGrouping::from_step(tick, d("0.25")),
        Err(PriceGroupingError::NotTickMultiple)
    );
    assert_eq!(
        PriceGrouping::new(tick, 0),
        Err(PriceGroupingError::ZeroMultiple)
    );
    assert!(TickSize::new(d("0")).is_err());
    assert!(TickSize::new(d("-0.1")).is_err());
}

#[test]
fn prices_fall_into_the_row_whose_low_edge_is_at_or_below_them() {
    let grouping = PriceGrouping::new(TickSize::new(d("0.1")).unwrap(), 5).unwrap();
    let cases = [
        ("100.0", "100.0"),
        ("100.4", "100.0"),
        ("100.5", "100.5"),
        ("99.9", "99.5"),
        ("0.3", "0.0"),
    ];
    for (price, low) in cases {
        let row = grouping.row_of(d(price));
        assert_eq!(grouping.row_low(row), d(low), "price {price}");
    }
}

#[test]
fn adjacent_rows_are_one_step_apart() {
    let grouping = PriceGrouping::new(TickSize::new(d("0.5")).unwrap(), 2).unwrap();
    let row = grouping.row_of(d("100"));
    assert_eq!(grouping.row_low(row.above()), d("101"));
    assert_eq!(grouping.row_low(row.below()), d("99"));
}

#[test]
fn a_finer_grouping_can_be_coarsened_only_to_a_multiple_of_its_step() {
    let tick = TickSize::new(d("0.1")).unwrap();
    let fine = PriceGrouping::new(tick, 2).unwrap();

    assert!(fine.can_coarsen_to(&PriceGrouping::new(tick, 10).unwrap()));
    assert!(!fine.can_coarsen_to(&PriceGrouping::new(tick, 5).unwrap()));
    assert!(!fine.can_coarsen_to(&PriceGrouping::new(tick, 1).unwrap()));
}

#[test]
fn timeframes_divide_the_trading_day_so_bars_never_straddle_it() {
    for secs in [60, 300, 900, 3_600, 14_400, 86_400] {
        assert!(Timeframe::from_secs(secs).is_ok(), "{secs}s");
    }
    assert_eq!(
        Timeframe::from_secs(0),
        Err(TimeframeError::NotDivisorOfDay)
    );
    assert_eq!(
        Timeframe::from_secs(7),
        Err(TimeframeError::NotDivisorOfDay)
    );
    assert_eq!(
        Timeframe::from_secs(172_800),
        Err(TimeframeError::NotDivisorOfDay)
    );
}

#[test]
fn a_timestamp_belongs_to_the_bar_that_starts_at_or_before_it() {
    let minute = Timeframe::from_secs(60).unwrap();
    let bar_start = Timestamp::from_micros(1_731_110_460_000_000);

    assert_eq!(minute.bar_start(bar_start), bar_start);
    assert_eq!(
        minute.bar_start(Timestamp::from_micros(1_731_110_519_999_999)),
        bar_start
    );
    assert_eq!(
        minute.bar_range(bar_start),
        TimeRange::new(bar_start, Timestamp::from_micros(1_731_110_520_000_000)).unwrap()
    );
    assert_eq!(
        minute.bar_start(Timestamp::from_micros(-1)),
        Timestamp::from_micros(-60_000_000)
    );
}

#[test]
fn time_ranges_are_half_open_and_non_empty() {
    let t = Timestamp::from_micros;
    let range = TimeRange::new(t(10), t(20)).unwrap();

    assert!(range.contains(t(10)));
    assert!(!range.contains(t(20)));
    assert!(TimeRange::new(t(20), t(20)).is_none());
    assert!(TimeRange::new(t(20), t(10)).is_none());
}
