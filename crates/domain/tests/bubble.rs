mod common;

use common::d;
use orderflow_domain::{
    Bubble, BubbleBuilder, BubbleGrouping, Decimal, LargeBubbleError, LargeBubbleRule, TakerSide,
    Timestamp, Trade, bubbles,
};
use proptest::prelude::*;

fn trade(id: u64, time_ms: i64, taker: TakerSide, price: &str, size: &str) -> Trade {
    Trade {
        venue_trade_id: id,
        engine_time: Timestamp::from_millis(time_ms),
        price: d(price),
        size: d(size),
        taker_side: taker,
        taker_order_id: None,
    }
}

fn buy(id: u64, time_ms: i64, price: &str, size: &str) -> Trade {
    trade(id, time_ms, TakerSide::Buy, price, size)
}

fn sell(id: u64, time_ms: i64, price: &str, size: &str) -> Trade {
    trade(id, time_ms, TakerSide::Sell, price, size)
}

fn binance_perp(trades: &[Trade]) -> Vec<Bubble> {
    bubbles(trades, BubbleGrouping::BINANCE_PERP)
}

fn shape(b: &Bubble) -> (TakerSide, u64, u64, u32) {
    (b.side, b.first_trade_id, b.last_trade_id, b.trade_count)
}

#[test]
fn a_same_side_run_inside_the_window_is_one_bubble_at_its_vwap() {
    let found = binance_perp(&[
        buy(1, 1_000, "100", "1"),
        buy(2, 1_040, "101", "2"),
        buy(3, 1_099, "102", "1"),
    ]);

    assert_eq!(found.len(), 1);
    let bubble = &found[0];
    assert_eq!(shape(bubble), (TakerSide::Buy, 1, 3, 3));
    assert_eq!(bubble.start, Timestamp::from_millis(1_000));
    assert_eq!(bubble.end, Timestamp::from_millis(1_099));
    assert_eq!(bubble.size, d("4"));
    assert_eq!(bubble.notional, d("404"));
    assert_eq!(bubble.vwap(), d("101"));
    assert_eq!((bubble.low, bubble.high), (d("100"), d("102")));
}

#[test]
fn a_change_of_taker_side_closes_the_bubble() {
    let found = binance_perp(&[
        buy(1, 1_000, "100", "1"),
        buy(2, 1_001, "100", "1"),
        sell(3, 1_002, "99.9", "2"),
        buy(4, 1_003, "100", "1"),
    ]);

    assert_eq!(
        found.iter().map(shape).collect::<Vec<_>>(),
        vec![
            (TakerSide::Buy, 1, 2, 2),
            (TakerSide::Sell, 3, 3, 1),
            (TakerSide::Buy, 4, 4, 1),
        ]
    );
}

#[test]
fn a_trade_100_ms_or_more_after_the_first_starts_a_new_bubble() {
    let found = binance_perp(&[
        buy(1, 1_000, "100", "1"),
        buy(2, 1_099, "100", "1"),
        buy(3, 1_100, "100", "1"),
        buy(4, 1_150, "100", "1"),
    ]);

    assert_eq!(
        found.iter().map(shape).collect::<Vec<_>>(),
        vec![(TakerSide::Buy, 1, 2, 2), (TakerSide::Buy, 3, 4, 2)]
    );
}

#[test]
fn a_jump_in_venue_trade_ids_closes_the_bubble() {
    let found = binance_perp(&[
        buy(1, 1_000, "100", "1"),
        buy(2, 1_001, "100", "1"),
        buy(4, 1_002, "100", "1"),
    ]);

    assert_eq!(
        found.iter().map(shape).collect::<Vec<_>>(),
        vec![(TakerSide::Buy, 1, 2, 2), (TakerSide::Buy, 4, 4, 1)]
    );
}

#[test]
fn a_builder_emits_each_bubble_when_the_next_trade_breaks_it_and_the_last_on_close() {
    let mut builder = BubbleBuilder::new(BubbleGrouping::BINANCE_PERP);

    assert_eq!(builder.push(&buy(1, 1_000, "100", "1")), None);
    assert_eq!(builder.push(&buy(2, 1_010, "100", "1")), None);
    let closed = builder.push(&sell(3, 1_020, "99", "1"));
    assert_eq!(closed.as_ref().map(shape), Some((TakerSide::Buy, 1, 2, 2)));
    assert_eq!(
        builder.close().as_ref().map(shape),
        Some((TakerSide::Sell, 3, 3, 1))
    );
    assert_eq!(builder.close(), None);
}

#[test]
fn a_custom_window_groups_by_its_own_length() {
    let trades = [buy(1, 1_000, "100", "1"), buy(2, 1_400, "100", "1")];

    assert_eq!(bubbles(&trades, BubbleGrouping::burst_millis(500)).len(), 1);
    assert_eq!(bubbles(&trades, BubbleGrouping::burst_millis(400)).len(), 2);
}

#[test]
fn the_large_bubble_threshold_is_the_nearest_rank_percentile_of_notional() {
    let rule = LargeBubbleRule::default();
    let mut notionals: Vec<Decimal> = (1..=200).map(Decimal::from).collect();

    assert_eq!(rule.threshold_index(200), Some(198));
    assert_eq!(rule.threshold_index(100), Some(99));
    assert_eq!(rule.threshold_index(1), Some(0));
    assert_eq!(rule.threshold_index(0), None);

    let threshold = rule.threshold(&mut notionals);
    assert_eq!(threshold, Some(d("199")));
    let large: Vec<_> = (1..=200)
        .map(Decimal::from)
        .filter(|n| rule.is_large(*n, threshold))
        .collect();
    assert_eq!(large, vec![d("199"), d("200")]);
}

#[test]
fn a_usd_floor_raises_the_bar_for_large_bubbles() {
    let rule = LargeBubbleRule::new(d("90"), Some(d("195"))).unwrap();
    let mut notionals: Vec<Decimal> = (1..=200).map(Decimal::from).collect();
    let threshold = rule.threshold(&mut notionals);

    assert_eq!(threshold, Some(d("181")));
    assert!(!rule.is_large(d("190"), threshold));
    assert!(rule.is_large(d("195"), threshold));
    assert!(!rule.is_large(d("1000"), None));
}

#[test]
fn a_percentile_must_be_strictly_between_0_and_100_and_a_floor_not_negative() {
    for bad in ["0", "100", "101", "-1"] {
        assert_eq!(
            LargeBubbleRule::new(d(bad), None),
            Err(LargeBubbleError::Percentile),
            "{bad}"
        );
    }
    assert_eq!(
        LargeBubbleRule::new(d("99"), Some(d("-5"))),
        Err(LargeBubbleError::Floor)
    );
    assert!(LargeBubbleRule::new(d("99.5"), Some(d("0"))).is_ok());
}

fn arbitrary_trades() -> impl Strategy<Value = Vec<Trade>> {
    prop::collection::vec((0_i64..300, any::<bool>(), 1_u32..500, 0_u64..3), 0..200).prop_map(
        |steps| {
            let mut id = 0_u64;
            let mut time_ms = 0_i64;
            steps
                .into_iter()
                .map(|(advance_ms, is_buy, size_milli, id_jump)| {
                    id += 1 + id_jump;
                    time_ms += advance_ms;
                    Trade {
                        venue_trade_id: id,
                        engine_time: Timestamp::from_millis(time_ms),
                        price: Decimal::new(100_000 + i64::from(size_milli % 7), 1),
                        size: Decimal::new(i64::from(size_milli), 3),
                        taker_side: if is_buy {
                            TakerSide::Buy
                        } else {
                            TakerSide::Sell
                        },
                        taker_order_id: None,
                    }
                })
                .collect()
        },
    )
}

proptest! {
    #[test]
    fn bubbles_partition_the_trades_exactly(trades in arbitrary_trades()) {
        let found = binance_perp(&trades);

        let count: u32 = found.iter().map(|b| b.trade_count).sum();
        prop_assert_eq!(usize::try_from(count).unwrap(), trades.len());
        for side in [TakerSide::Buy, TakerSide::Sell] {
            let bubble_size: Decimal = found.iter().filter(|b| b.side == side).map(|b| b.size).sum();
            let trade_size: Decimal = trades.iter().filter(|t| t.taker_side == side).map(|t| t.size).sum();
            prop_assert_eq!(bubble_size, trade_size);
            let bubble_notional: Decimal = found.iter().filter(|b| b.side == side).map(|b| b.notional).sum();
            let trade_notional: Decimal = trades.iter().filter(|t| t.taker_side == side).map(Trade::notional).sum();
            prop_assert_eq!(bubble_notional, trade_notional);
        }
        for bubble in &found {
            prop_assert!(bubble.end.as_micros() - bubble.start.as_micros() < 100_000);
            prop_assert!(bubble.low <= bubble.vwap() && bubble.vwap() <= bubble.high);
        }
    }
}
