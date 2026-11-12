use orderflow_domain::{
    Bubble, Decimal, Gap, GapReason, LargeBubbleRule, Market, Ohlc, PriceGrouping, TakerSide,
    TickSize, TimeRange, Timeframe, Timestamp, Trade,
};
use orderflow_store::{
    BubbleBatchRecord, BubbleQuery, GapEvent, Store, StoreConfig, StoreError, TradeBatchRecord,
    WriteBatch,
};

const MIN: i64 = 60_000_000;
const T0: i64 = 1_794_179_700_000_000; // a minute boundary

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn btc() -> Market {
    "binance:perp:BTCUSDT".parse().unwrap()
}

async fn store(test: &str) -> Store {
    let config = StoreConfig {
        database: format!("of_test_{test}_{}", std::process::id()),
        ..StoreConfig::local()
    };
    let store = Store::connect(&config).await.unwrap();
    store.drop_database_for_tests().await.unwrap();
    Store::connect(&config).await.unwrap()
}

fn trade(id: u64, t: i64, price: &str, size: &str, taker: TakerSide) -> Trade {
    Trade {
        venue_trade_id: id,
        engine_time: Timestamp::from_micros(t),
        price: d(price),
        size: d(size),
        taker_side: taker,
        taker_order_id: None,
    }
}

fn batch(seq: u64, trades: Vec<Trade>) -> TradeBatchRecord {
    TradeBatchRecord {
        market: btc(),
        trades,
        stream_seq: seq,
    }
}

fn range(start: i64, end: i64) -> TimeRange {
    TimeRange::new(Timestamp::from_micros(start), Timestamp::from_micros(end)).unwrap()
}

fn grouping(tick: &str, multiple: u32) -> PriceGrouping {
    PriceGrouping::new(TickSize::new(d(tick)).unwrap(), multiple).unwrap()
}

#[tokio::test]
async fn connecting_twice_applies_migrations_once() {
    let store = store("migrate").await;
    assert_eq!(store.high_water_mark().await.unwrap(), 0);
}

#[tokio::test]
async fn footprint_bars_are_built_from_stored_trades_at_any_nesting_grouping() {
    let store = store("footprint").await;
    store
        .write(&WriteBatch {
            trade_batches: vec![
                batch(
                    1,
                    vec![
                        trade(1, T0, "100.1", "1.5", TakerSide::Buy),
                        trade(2, T0 + 1, "100.4", "2", TakerSide::Sell),
                    ],
                ),
                batch(
                    2,
                    vec![
                        trade(3, T0 + MIN, "101.0", "0.25", TakerSide::Buy),
                        trade(4, T0 + 5 * MIN, "99.9", "8", TakerSide::Sell),
                    ],
                ),
            ],
            gap_events: vec![],
            ..WriteBatch::default()
        })
        .await
        .unwrap();

    let minute = Timeframe::from_secs(60).unwrap();
    let bars = store
        .footprint_bars(&btc(), range(T0, T0 + 10 * MIN), minute, grouping("0.1", 5))
        .await
        .unwrap();
    let summary: Vec<_> = bars
        .iter()
        .map(|b| {
            (
                b.range().start().as_micros(),
                b.rows().map(|r| (r.low, r.buy, r.sell)).collect::<Vec<_>>(),
                b.trade_count(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            (T0, vec![(d("100.0"), d("1.5"), d("2"))], 2),
            (T0 + MIN, vec![(d("101.0"), d("0.25"), d("0"))], 1),
            (T0 + 5 * MIN, vec![(d("99.5"), d("0"), d("8"))], 1),
        ]
    );

    let five = store
        .footprint_bars(
            &btc(),
            range(T0, T0 + 10 * MIN),
            Timeframe::from_secs(300).unwrap(),
            grouping("0.1", 10),
        )
        .await
        .unwrap();
    assert_eq!(five.len(), 2);
    assert_eq!(five[0].buy_volume(), d("1.75"));
    assert_eq!(five[0].sell_volume(), d("2"));
    assert_eq!(five[0].trade_count(), 3);
}

#[tokio::test]
async fn timeframes_shorter_than_or_not_a_multiple_of_a_minute_are_refused() {
    let store = store("timeframe").await;
    let result = store
        .footprint_bars(
            &btc(),
            range(T0, T0 + MIN),
            Timeframe::from_secs(30).unwrap(),
            grouping("0.1", 1),
        )
        .await;
    assert!(matches!(result, Err(StoreError::UnsupportedTimeframe)));
}

#[tokio::test]
async fn gaps_pair_open_and_close_events_and_duplicates_are_harmless() {
    let store = store("gaps").await;
    let open = |id: &str, t: i64, reason| GapEvent::Opened {
        market: btc(),
        gap_id: id.into(),
        start: Timestamp::from_micros(t),
        reason,
    };
    let close = |id: &str, t: i64| GapEvent::Closed {
        market: btc(),
        gap_id: id.into(),
        end: Timestamp::from_micros(t),
    };
    let events = vec![
        (open("a", T0, GapReason::Disconnect), 1),
        (close("a", T0 + 100), 2),
        (open("a", T0, GapReason::Disconnect), 3), // redelivered
        (open("b", T0 + MIN, GapReason::SequenceBreak), 4),
        (open("c", T0 + 9 * MIN, GapReason::StorageOutage), 5),
    ];
    store
        .write(&WriteBatch {
            trade_batches: vec![],
            gap_events: events,
            ..WriteBatch::default()
        })
        .await
        .unwrap();

    let gaps = store.gaps(&btc(), range(T0, T0 + 2 * MIN)).await.unwrap();

    assert_eq!(
        gaps,
        vec![
            Gap::closed(
                Timestamp::from_micros(T0),
                Timestamp::from_micros(T0 + 100),
                GapReason::Disconnect
            )
            .unwrap(),
            Gap::open(Timestamp::from_micros(T0 + MIN), GapReason::SequenceBreak),
        ]
    );
}

#[tokio::test]
async fn recording_horizon_and_high_water_mark_come_from_stored_trades() {
    let store = store("horizon").await;
    assert_eq!(store.recording_horizon(&btc()).await.unwrap(), None);

    store
        .write(&WriteBatch {
            trade_batches: vec![
                batch(7, vec![trade(1, T0 + 5, "1", "1", TakerSide::Buy)]),
                batch(9, vec![trade(2, T0 + 9, "1", "1", TakerSide::Buy)]),
            ],
            gap_events: vec![],
            ..WriteBatch::default()
        })
        .await
        .unwrap();

    assert_eq!(
        store.recording_horizon(&btc()).await.unwrap(),
        Some(Timestamp::from_micros(T0 + 5))
    );
    assert_eq!(store.high_water_mark().await.unwrap(), 9);
    assert_eq!(store.markets().await.unwrap(), vec![btc()]);
}

#[tokio::test]
async fn decimals_with_more_than_18_places_are_refused_not_rounded() {
    let store = store("precision").await;
    let result = store
        .write(&WriteBatch {
            trade_batches: vec![batch(
                1,
                vec![trade(1, T0, "0.0000000000000000001", "1", TakerSide::Buy)],
            )],
            gap_events: vec![],
            ..WriteBatch::default()
        })
        .await;
    assert!(matches!(result, Err(StoreError::Precision(_))));
}

fn bubble(first_id: u64, start: i64, side: TakerSide, notional: &str) -> Bubble {
    Bubble {
        side,
        start: Timestamp::from_micros(start),
        end: Timestamp::from_micros(start + 40_000),
        first_trade_id: first_id,
        last_trade_id: first_id + 1,
        trade_count: 2,
        size: d("1"),
        notional: d(notional),
        low: d(notional),
        high: d(notional),
    }
}

fn bubble_batch(seq: u64, bubbles: Vec<Bubble>) -> BubbleBatchRecord {
    BubbleBatchRecord {
        market: btc(),
        bubbles,
        stream_seq: seq,
    }
}

#[tokio::test]
async fn bubbles_are_filtered_by_notional_and_side_and_rewriting_them_is_harmless() {
    let store = store("bubbles").await;
    let stored = vec![
        bubble(10, T0, TakerSide::Buy, "500"),
        bubble(12, T0 + 1_000, TakerSide::Sell, "75000.25"),
        bubble(14, T0 + 2_000, TakerSide::Buy, "120000"),
        bubble(16, T0 + MIN, TakerSide::Buy, "90000"),
    ];
    let batch = WriteBatch {
        bubble_batches: vec![bubble_batch(1, stored.clone())],
        ..WriteBatch::default()
    };
    store.write(&batch).await.unwrap();
    store.write(&batch).await.unwrap();

    let all = BubbleQuery::default();
    let page = store
        .bubbles(&btc(), range(T0, T0 + MIN), &all)
        .await
        .unwrap();
    assert_eq!(page.bubbles, stored[..3].to_vec());
    assert!(!page.truncated);

    let big_buys = BubbleQuery {
        min_notional: d("1000"),
        side: Some(TakerSide::Buy),
        ..BubbleQuery::default()
    };
    let page = store
        .bubbles(&btc(), range(T0, T0 + 2 * MIN), &big_buys)
        .await
        .unwrap();
    assert_eq!(page.bubbles, vec![stored[2].clone(), stored[3].clone()]);

    let largest_two = BubbleQuery {
        limit: 2,
        ..BubbleQuery::default()
    };
    let page = store
        .bubbles(&btc(), range(T0, T0 + 2 * MIN), &largest_two)
        .await
        .unwrap();
    assert_eq!(page.bubbles, vec![stored[2].clone(), stored[3].clone()]);
    assert!(page.truncated);
}

#[tokio::test]
async fn the_large_bubble_threshold_in_storage_follows_the_domain_rule() {
    let store = store("largebubble").await;
    let window = range(T0, T0 + 200 * 1_000);
    let rule = LargeBubbleRule::default();
    assert_eq!(
        store
            .large_bubble_threshold(&btc(), window, &rule)
            .await
            .unwrap(),
        None
    );

    let bubbles: Vec<Bubble> = (1..=200_u64)
        .map(|n| {
            bubble(
                n * 2,
                T0 + i64::try_from(n).unwrap() * 1_000 - 1_000,
                TakerSide::Buy,
                &n.to_string(),
            )
        })
        .rev()
        .collect();
    store
        .write(&WriteBatch {
            bubble_batches: vec![bubble_batch(1, bubbles.clone())],
            ..WriteBatch::default()
        })
        .await
        .unwrap();

    let mut notionals: Vec<Decimal> = bubbles.iter().map(|b| b.notional).collect();
    for rule in [
        LargeBubbleRule::default(),
        LargeBubbleRule::new(d("90"), None).unwrap(),
        LargeBubbleRule::new(d("99.5"), None).unwrap(),
        LargeBubbleRule::new(d("12.3"), None).unwrap(),
    ] {
        assert_eq!(
            store
                .large_bubble_threshold(&btc(), window, &rule)
                .await
                .unwrap(),
            rule.threshold(&mut notionals),
            "{rule:?}"
        );
    }
}

#[tokio::test]
async fn taker_delta_over_a_span_comes_from_stored_trades() {
    let store = store("delta").await;
    store
        .write(&WriteBatch {
            trade_batches: vec![batch(
                1,
                vec![
                    trade(1, T0, "100", "2.5", TakerSide::Buy),
                    trade(2, T0 + 10, "100", "1", TakerSide::Sell),
                    trade(3, T0 + MIN, "100", "4", TakerSide::Sell),
                    trade(4, T0 + 2 * MIN, "100", "9", TakerSide::Buy),
                ],
            )],
            ..WriteBatch::default()
        })
        .await
        .unwrap();

    assert_eq!(
        store
            .taker_delta(&btc(), range(T0, T0 + 2 * MIN))
            .await
            .unwrap(),
        d("-2.5")
    );
    assert_eq!(
        store
            .taker_delta(&btc(), range(T0, T0 + MIN))
            .await
            .unwrap(),
        d("1.5")
    );
    assert_eq!(
        store
            .taker_delta(&btc(), range(T0 - MIN, T0))
            .await
            .unwrap(),
        d("0")
    );
}

#[tokio::test]
async fn footprint_bars_carry_exact_ohlc_across_minutes() {
    let store = store("ohlc").await;
    store
        .write(&WriteBatch {
            trade_batches: vec![batch(
                1,
                vec![
                    trade(1, T0, "100.0", "1", TakerSide::Buy),
                    trade(2, T0 + 1_000, "103.5", "1", TakerSide::Sell),
                    trade(3, T0 + 2_000, "99.0", "1", TakerSide::Buy),
                    trade(4, T0 + MIN, "101.0", "1", TakerSide::Buy),
                    trade(5, T0 + MIN + 1_000, "98.5", "1", TakerSide::Sell),
                    trade(6, T0 + MIN + 2_000, "102.0", "1", TakerSide::Buy),
                    trade(7, T0 + MIN + 2_000, "102.5", "1", TakerSide::Buy),
                ],
            )],
            gap_events: vec![],
            ..WriteBatch::default()
        })
        .await
        .unwrap();

    let bars = store
        .footprint_bars(
            &btc(),
            range(T0, T0 + 5 * MIN),
            Timeframe::from_secs(300).unwrap(),
            grouping("0.1", 10),
        )
        .await
        .unwrap();

    assert_eq!(bars.len(), 1);
    assert_eq!(
        bars[0].ohlc(),
        Some(Ohlc {
            open: d("100.0"),
            high: d("103.5"),
            low: d("98.5"),
            close: d("102.5"),
        })
    );
}
