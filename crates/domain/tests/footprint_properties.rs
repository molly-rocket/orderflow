use orderflow_domain::{
    Decimal, FootprintBar, PriceGrouping, TakerSide, TickSize, Timeframe, Timestamp, Trade,
    footprint_bars,
};
use proptest::prelude::*;

const DAY_START: i64 = 1_731_110_400_000_000;

fn arb_trades() -> impl Strategy<Value = Vec<Trade>> {
    let trade = (
        0_i64..3_600_000_000,
        1_000_000_i64..10_000_000,
        1_i64..10_000_000,
        any::<bool>(),
    );
    prop::collection::vec(trade, 0..300).prop_map(|raw| {
        let mut trades: Vec<Trade> = raw
            .into_iter()
            .map(|(offset, price_cents, size_micro, is_buy)| Trade {
                venue_trade_id: 0,
                engine_time: Timestamp::from_micros(DAY_START + offset),
                price: Decimal::new(price_cents, 2),
                size: Decimal::new(size_micro, 6),
                taker_side: if is_buy {
                    TakerSide::Buy
                } else {
                    TakerSide::Sell
                },
                taker_order_id: None,
            })
            .collect();
        trades.sort_by_key(|t| t.engine_time);
        for (id, trade) in (0_u64..).zip(&mut trades) {
            trade.venue_trade_id = id;
        }
        trades
    })
}

fn tick() -> TickSize {
    TickSize::new(Decimal::new(1, 2)).unwrap()
}

fn side_total(trades: &[Trade], side: TakerSide) -> Decimal {
    trades
        .iter()
        .filter(|t| t.taker_side == side)
        .map(|t| t.size)
        .sum()
}

proptest! {
    #[test]
    fn footprint_rows_sum_exactly_to_the_trades(
        trades in arb_trades(),
        multiple in 1_u32..500,
        timeframe_secs in prop::sample::select(vec![60_u32, 300, 900, 3_600]),
    ) {
        let grouping = PriceGrouping::new(tick(), multiple).unwrap();
        let bars = footprint_bars(&trades, Timeframe::from_secs(timeframe_secs).unwrap(), grouping);

        let buys: Decimal = bars.iter().flat_map(FootprintBar::rows).map(|r| r.buy).sum();
        let sells: Decimal = bars.iter().flat_map(FootprintBar::rows).map(|r| r.sell).sum();
        let count: u64 = bars.iter().map(FootprintBar::trade_count).sum();

        prop_assert_eq!(buys, side_total(&trades, TakerSide::Buy));
        prop_assert_eq!(sells, side_total(&trades, TakerSide::Sell));
        prop_assert_eq!(count, trades.len() as u64);
        for bar in &bars {
            prop_assert!(!bar.is_empty());
            prop_assert_eq!(bar.delta(), bar.buy_volume() - bar.sell_volume());
        }
    }

    #[test]
    fn regrouping_fine_to_coarse_equals_building_coarse_directly(
        trades in arb_trades(),
        fine_multiple in 1_u32..50,
        factor in 2_u32..20,
    ) {
        let minute = Timeframe::from_secs(60).unwrap();
        let fine = PriceGrouping::new(tick(), fine_multiple).unwrap();
        let coarse = PriceGrouping::new(tick(), fine_multiple * factor).unwrap();

        let regrouped: Vec<_> = footprint_bars(&trades, minute, fine)
            .iter()
            .map(|bar| bar.regroup(coarse).unwrap())
            .collect();

        prop_assert_eq!(regrouped, footprint_bars(&trades, minute, coarse));
    }
}
