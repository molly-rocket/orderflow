use orderflow_contracts::subjects::{self, Namespace};
use orderflow_contracts::v1::{Decimal, GapOpened, TakerSide, Trade, TradeBatch};
use orderflow_domain::Market;
use prost::Message;

fn market(id: &str) -> Market {
    id.parse().unwrap()
}

#[test]
fn trade_batch_survives_a_wire_round_trip() {
    let batch = TradeBatch {
        market: "binance:perp:BTCUSDT".into(),
        trades: vec![Trade {
            venue_trade_id: 42,
            engine_time_us: 1_731_110_400_123_000,
            price: Some(Decimal {
                mantissa: 7_654_321,
                scale: 1,
            }),
            size: Some(Decimal {
                mantissa: -1,
                scale: 3,
            }),
            taker_side: TakerSide::Sell.into(),
            taker_order_id: None,
        }],
    };

    let decoded = TradeBatch::decode(batch.encode_to_vec().as_slice()).unwrap();

    assert_eq!(decoded, batch);
}

#[test]
fn message_type_names_carry_the_schema_version() {
    assert_eq!(
        subjects::type_name::<TradeBatch>(),
        "orderflow.v1.TradeBatch"
    );
    assert_eq!(subjects::type_name::<GapOpened>(), "orderflow.v1.GapOpened");
}

#[test]
fn record_subjects_are_versioned_and_tokenise_the_market_id() {
    let ns = Namespace::production();
    assert_eq!(
        ns.trades(&market("binance:perp:BTCUSDT")),
        "of.v1.trade.binance.perp.BTCUSDT"
    );
    assert_eq!(
        ns.gaps(&market("hyperliquid:spot:PURR/USDC")),
        "of.v1.gap.hyperliquid.spot.PURR/USDC"
    );
}

#[test]
fn a_native_name_containing_a_colon_stays_one_token() {
    // Hyperliquid HIP-3 perps are named `dex:COIN`.
    assert_eq!(
        Namespace::production().trades(&market("hyperliquid:perp:xyz:TSLA")),
        "of.v1.trade.hyperliquid.perp.xyz:TSLA"
    );
}

#[test]
fn the_production_record_stream_captures_trades_gaps_and_bubbles_for_every_market() {
    let ns = Namespace::production();
    assert_eq!(ns.record_stream(), "OF_RECORD_V1");
    assert_eq!(
        ns.record_stream_subjects(),
        ["of.v1.trade.>", "of.v1.gap.>", "of.v1.bubble.>"]
    );
    assert_eq!(
        ns.bubbles(&market("binance:perp:BTCUSDT")),
        "of.v1.bubble.binance.perp.BTCUSDT"
    );
    assert_eq!(ns.all_trades(), "of.v1.trade.>");
    assert_eq!(ns.kv_bucket("collector"), "of_collector_v1");
}

#[test]
fn other_namespaces_are_fully_isolated_from_production() {
    let ns = Namespace::new("e2e_42").unwrap();
    assert_eq!(
        ns.trades(&market("binance:perp:BTCUSDT")),
        "e2e_42.v1.trade.binance.perp.BTCUSDT"
    );
    assert_eq!(ns.record_stream(), "E2E_42_RECORD_V1");
    assert_eq!(ns.kv_bucket("collector"), "e2e_42_collector_v1");
    for bad in ["", "Of", "a.b", "a b", "a>", "a*"] {
        assert!(Namespace::new(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn book_state_round_trips_through_protobuf() {
    use orderflow_contracts::book::{BookState, book_state_from_proto, book_state_to_proto};
    use orderflow_domain::{BookStatus, Level, Timestamp};

    let lvl = |p: &str, s: &str| Level {
        price: p.parse().unwrap(),
        size: s.parse().unwrap(),
    };
    let state = BookState {
        market: market("binance:perp:BTCUSDT"),
        time: Timestamp::from_micros(1_700_000_000_000_000),
        status: BookStatus::Live,
        bids: vec![lvl("99.9", "2"), lvl("99.8", "1.5")],
        asks: vec![lvl("100.0", "3"), lvl("100.2", "4")],
        known: Some(("99.8".parse().unwrap(), "100.2".parse().unwrap())),
        best_bid: Some(lvl("99.95", "0.5")),
        best_ask: Some(lvl("100.0", "3")),
    };

    let wire = book_state_to_proto(&state).unwrap().encode_to_vec();
    let decoded = orderflow_contracts::v1::BookState::decode(wire.as_slice()).unwrap();

    assert_eq!(book_state_from_proto(decoded).unwrap(), state);
}

#[test]
fn a_non_live_book_state_round_trips_with_no_levels_or_range() {
    use orderflow_contracts::book::{BookState, book_state_from_proto, book_state_to_proto};
    use orderflow_domain::{BookStatus, Timestamp};

    let state = BookState {
        market: market("binance:perp:BTCUSDT"),
        time: Timestamp::from_micros(1),
        status: BookStatus::Resyncing,
        bids: vec![],
        asks: vec![],
        known: None,
        best_bid: None,
        best_ask: None,
    };
    assert_eq!(
        book_state_from_proto(book_state_to_proto(&state).unwrap()).unwrap(),
        state
    );
}

#[test]
fn book_and_heatmap_subjects_are_not_recorded() {
    let ns = Namespace::production();
    let btc = market("binance:perp:BTCUSDT");
    assert_eq!(ns.book(&btc), "of.v1.book.binance.perp.BTCUSDT");
    assert_eq!(ns.heatmap(&btc), "of.v1.heatmap.binance.perp.BTCUSDT");
    assert!(
        !ns.record_stream_subjects()
            .iter()
            .any(|s| s.contains(".book.") || s.contains(".heatmap."))
    );
}
