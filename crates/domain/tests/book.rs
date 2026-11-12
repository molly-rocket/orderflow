mod common;

use common::d;
use orderflow_domain::{
    BookSide, Decimal, Ladder, LadderRow, Level, OrderBook, PriceGrouping, PriceRow, TickSize,
};

fn lvl(price: &str, size: &str) -> Level {
    Level {
        price: d(price),
        size: d(size),
    }
}

fn grouping(tick: &str, multiple: u32) -> PriceGrouping {
    PriceGrouping::new(TickSize::new(d(tick)).unwrap(), multiple).unwrap()
}

fn book() -> OrderBook {
    OrderBook::from_snapshot(
        &[lvl("99.9", "2"), lvl("99.8", "1")],
        &[lvl("100.0", "3"), lvl("100.2", "4")],
    )
}

#[test]
fn applying_diffs_updates_and_removes_levels() {
    let mut book = book();

    book.apply(BookSide::Bid, d("99.9"), d("5"));
    book.apply(BookSide::Ask, d("100.0"), Decimal::ZERO);

    assert_eq!(book.best_bid(), Some(lvl("99.9", "5")));
    assert_eq!(book.best_ask(), Some(lvl("100.2", "4")));
    assert_eq!(book.mid(), Some(d("100.05")));
    assert_eq!(book.known_range(), Some((d("99.8"), d("100.2"))));
}

#[test]
fn levels_outside_the_known_range_are_not_reported() {
    let mut book = book();

    book.apply(BookSide::Ask, d("105.0"), d("9"));
    book.apply(BookSide::Bid, d("90.0"), d("9"));

    assert_eq!(
        book.levels(BookSide::Ask),
        vec![lvl("100.0", "3"), lvl("100.2", "4")]
    );
    assert_eq!(
        book.levels(BookSide::Bid),
        vec![lvl("99.9", "2"), lvl("99.8", "1")]
    );
}

#[test]
fn an_empty_side_in_the_snapshot_means_no_known_range() {
    let book = OrderBook::from_snapshot(&[lvl("99.9", "2")], &[]);

    assert_eq!(book.known_range(), None);
    assert_eq!(book.best_ask(), None);
}

#[test]
fn a_ladder_buckets_by_grouping_and_drops_partially_known_rows() {
    let bids = [lvl("100.3", "3"), lvl("99.9", "7")];
    let asks = [lvl("100.6", "4"), lvl("101.2", "8")];

    let ladder = Ladder::new(
        &bids,
        &asks,
        Some((d("99.5"), d("101.4"))),
        grouping("0.1", 10),
    );

    assert_eq!(ladder.step, d("1"));
    assert_eq!(
        ladder.known,
        Some((PriceRow::from_index(100), PriceRow::from_index(100)))
    );
    assert_eq!(
        ladder.rows,
        vec![LadderRow {
            row: PriceRow::from_index(100),
            low: d("100"),
            bid: d("3"),
            ask: d("4"),
        }]
    );
}

#[test]
fn a_ladder_at_tick_grouping_lists_every_sized_level_ascending() {
    let bids = [lvl("99.9", "2"), lvl("99.8", "1")];
    let asks = [lvl("100.2", "4"), lvl("100.0", "3")];

    let ladder = Ladder::new(
        &bids,
        &asks,
        Some((d("99.8"), d("100.2"))),
        grouping("0.1", 1),
    );

    let lows: Vec<Decimal> = ladder.rows.iter().map(|r| r.low).collect();
    assert_eq!(lows, vec![d("99.8"), d("99.9"), d("100"), d("100.2")]);
    assert_eq!(
        ladder.known,
        Some((PriceRow::from_index(998), PriceRow::from_index(1002)))
    );
}

#[test]
fn a_ladder_without_a_known_range_has_no_rows() {
    let ladder = Ladder::new(&[lvl("99.9", "2")], &[], None, grouping("0.1", 1));

    assert_eq!(ladder.known, None);
    assert!(ladder.rows.is_empty());
}
