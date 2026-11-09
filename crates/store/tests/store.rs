use orderflow_domain::{
    Decimal, Gap, GapReason, Market, PriceGrouping, TakerSide, TickSize, TimeRange, Timeframe,
    Timestamp, Trade,
};
use orderflow_store::{GapEvent, Store, StoreConfig, StoreError, TradeBatchRecord, WriteBatch};

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
        })
        .await;
    assert!(matches!(result, Err(StoreError::Precision(_))));
}
