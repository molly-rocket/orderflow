use orderflow_contracts::convert::{self, ConvertError};
use orderflow_contracts::v1;
use orderflow_domain::{Decimal, GapReason, Market, TakerSide, Timestamp, Trade};

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn domain_trade() -> Trade {
    Trade {
        venue_trade_id: 3_512_548_202,
        engine_time: Timestamp::from_millis(1_794_179_696_319),
        price: d("75191.30"),
        size: d("0.086"),
        taker_side: TakerSide::Sell,
        taker_order_id: Some("abc".into()),
    }
}

#[test]
fn decimals_convert_exactly_both_ways() {
    for s in ["0", "75191.30", "-0.000001", "0.086", "123456789.12345678"] {
        let proto = convert::decimal_to_proto(d(s)).unwrap();
        assert_eq!(convert::decimal_from_proto(proto).unwrap(), d(s), "{s}");
    }
    let proto = convert::decimal_to_proto(d("75191.30")).unwrap();
    assert_eq!((proto.mantissa, proto.scale), (7_519_130, 2));
}

#[test]
fn decimals_too_wide_for_the_wire_are_rejected() {
    let huge = Decimal::MAX;
    assert_eq!(
        convert::decimal_to_proto(huge),
        Err(ConvertError::DecimalOutOfRange)
    );
    assert_eq!(
        convert::decimal_from_proto(v1::Decimal {
            mantissa: 1,
            scale: 29
        }),
        Err(ConvertError::DecimalOutOfRange)
    );
}

#[test]
fn a_trade_batch_round_trips_through_the_wire_types() {
    let market: Market = "binance:perp:BTCUSDT".parse().unwrap();
    let trades = vec![domain_trade()];

    let proto = convert::trade_batch_to_proto(&market, &trades).unwrap();
    assert_eq!(proto.market, "binance:perp:BTCUSDT");
    assert_eq!(proto.trades[0].taker_side(), v1::TakerSide::Sell);

    let (back_market, back_trades) = convert::trade_batch_from_proto(proto).unwrap();
    assert_eq!(back_market, market);
    assert_eq!(back_trades, trades);
}

#[test]
fn malformed_wire_trades_are_rejected_not_guessed() {
    let mut proto =
        convert::trade_batch_to_proto(&"binance:perp:BTCUSDT".parse().unwrap(), &[domain_trade()])
            .unwrap();
    proto.trades[0].taker_side = v1::TakerSide::Unspecified.into();
    assert_eq!(
        convert::trade_batch_from_proto(proto.clone()).unwrap_err(),
        ConvertError::MissingField("taker_side")
    );

    proto.trades[0].taker_side = v1::TakerSide::Buy.into();
    proto.trades[0].price = None;
    assert_eq!(
        convert::trade_batch_from_proto(proto.clone()).unwrap_err(),
        ConvertError::MissingField("price")
    );

    proto.market = "nope".into();
    assert!(matches!(
        convert::trade_batch_from_proto(proto).unwrap_err(),
        ConvertError::Market(_)
    ));
}

#[test]
fn gap_reasons_map_one_to_one() {
    for reason in [
        GapReason::Disconnect,
        GapReason::SequenceBreak,
        GapReason::BookResync,
        GapReason::RecorderDowntime,
        GapReason::StorageOutage,
    ] {
        let proto = convert::gap_reason_to_proto(reason);
        assert_eq!(convert::gap_reason_from_proto(proto).unwrap(), reason);
    }
    assert_eq!(
        convert::gap_reason_from_proto(v1::GapReason::Unspecified),
        Err(ConvertError::MissingField("reason"))
    );
}
