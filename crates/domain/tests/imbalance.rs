mod common;

use common::{buy, d, sell};
use orderflow_domain::{
    FootprintBar, Imbalance, ImbalanceError, ImbalanceSettings, PriceGrouping, TakerSide, TickSize,
    Timeframe, Timestamp, Trade, find_imbalances,
};

const T: i64 = 1_731_110_460_000_000;

fn bar(rows: &[(i64, &str, &str)]) -> FootprintBar {
    let grouping = PriceGrouping::new(TickSize::new(d("1")).unwrap(), 1).unwrap();
    let range = Timeframe::from_secs(60)
        .unwrap()
        .bar_range(Timestamp::from_micros(T));
    let mut bar = FootprintBar::new(range, grouping);
    let mut trades: Vec<Trade> = Vec::new();
    for &(price, b, s) in rows {
        let price = price.to_string();
        if d(b) > d("0") {
            trades.push(buy(T, &price, b));
        }
        if d(s) > d("0") {
            trades.push(sell(T, &price, s));
        }
    }
    for trade in &trades {
        bar.add(trade).unwrap();
    }
    bar
}

fn imbalances(bar: &FootprintBar, settings: &ImbalanceSettings) -> Vec<(i64, TakerSide)> {
    find_imbalances(bar, settings)
        .imbalances
        .iter()
        .map(|i: &Imbalance| (i.row.index(), i.side))
        .collect()
}

#[test]
fn default_settings_are_300_percent_and_stacks_of_three() {
    let s = ImbalanceSettings::default();
    assert_eq!(s.ratio(), d("3"));
    assert_eq!(s.min_stack(), 3);
}

#[test]
fn buy_imbalance_compares_buys_with_sells_one_row_below() {
    //            price  buy   sell
    let b = bar(&[(100, "0", "2"), (101, "6", "0")]);
    assert_eq!(
        imbalances(&b, &ImbalanceSettings::default()),
        vec![(101, TakerSide::Buy)]
    );

    let just_under = bar(&[(100, "0", "2"), (101, "5.99", "0")]);
    assert_eq!(
        imbalances(&just_under, &ImbalanceSettings::default()),
        vec![]
    );
}

#[test]
fn sell_imbalance_compares_sells_with_buys_one_row_above() {
    let b = bar(&[(100, "0", "9"), (101, "3", "0")]);
    assert_eq!(
        imbalances(&b, &ImbalanceSettings::default()),
        vec![(100, TakerSide::Sell)]
    );
}

#[test]
fn zero_on_the_diagonal_inside_the_bar_counts_as_an_imbalance() {
    let b = bar(&[(100, "0", "1"), (102, "1", "0")]);
    assert_eq!(
        imbalances(&b, &ImbalanceSettings::default()),
        vec![(100, TakerSide::Sell), (102, TakerSide::Buy)]
    );
}

#[test]
fn edge_rows_have_no_diagonal_outside_the_bar() {
    let b = bar(&[(100, "5", "0"), (101, "0", "5")]);
    assert_eq!(imbalances(&b, &ImbalanceSettings::default()), vec![]);

    let single = bar(&[(100, "5", "5")]);
    assert_eq!(imbalances(&single, &ImbalanceSettings::default()), vec![]);
}

#[test]
fn ratio_is_configurable_and_must_be_at_least_one() {
    let b = bar(&[(100, "0", "2"), (101, "3", "0")]);
    let s = ImbalanceSettings::new(d("1.5"), 3).unwrap();
    assert_eq!(imbalances(&b, &s), vec![(101, TakerSide::Buy)]);

    assert_eq!(
        ImbalanceSettings::new(d("0.9"), 3),
        Err(ImbalanceError::RatioBelowOne)
    );
    assert_eq!(
        ImbalanceSettings::new(d("3"), 1),
        Err(ImbalanceError::StackTooShort)
    );
}

#[test]
fn three_consecutive_same_side_imbalances_form_a_stack() {
    let b = bar(&[
        (100, "0", "1"),
        (101, "3", "1"),
        (102, "3", "1"),
        (103, "3", "0"),
    ]);
    let found = find_imbalances(&b, &ImbalanceSettings::default());

    assert_eq!(found.imbalances.len(), 3);
    assert_eq!(found.stacks.len(), 1);
    let stack = found.stacks[0];
    assert_eq!(stack.side, TakerSide::Buy);
    assert_eq!((stack.low.index(), stack.high.index()), (101, 103));
}

#[test]
fn two_consecutive_imbalances_are_not_a_stack() {
    let b = bar(&[(100, "0", "1"), (101, "3", "1"), (102, "3", "5")]);
    let found = find_imbalances(&b, &ImbalanceSettings::default());

    assert_eq!(found.imbalances.len(), 2);
    assert!(found.stacks.is_empty());
}

#[test]
fn a_stack_is_broken_by_a_gap_in_rows_or_a_change_of_side() {
    let b = bar(&[
        (100, "0", "1"),
        (101, "3", "1"),
        (102, "3", "1"),
        (103, "1", "1"),
        (104, "3", "1"),
        (105, "3", "1"),
        (106, "3", "1"),
    ]);
    let found = find_imbalances(&b, &ImbalanceSettings::default());

    let stacks: Vec<_> = found
        .stacks
        .iter()
        .map(|s| (s.side, s.low.index(), s.high.index()))
        .collect();
    assert_eq!(stacks, vec![(TakerSide::Buy, 104, 106)]);
}
