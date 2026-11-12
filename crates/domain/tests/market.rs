use orderflow_domain::{Decimal, Market, MarketIdError, MarketSpec, MarketType, Venue};

#[test]
fn parses_and_formats_a_market_id() {
    let market: Market = "binance:perp:BTCUSDT".parse().unwrap();

    assert_eq!(market.venue(), Venue::Binance);
    assert_eq!(market.market_type(), MarketType::Perp);
    assert_eq!(market.native_name(), "BTCUSDT");
    assert_eq!(market.to_string(), "binance:perp:BTCUSDT");
}

#[test]
fn native_names_keep_venue_specific_characters() {
    for id in [
        "hyperliquid:spot:PURR/USDC",
        "hyperliquid:spot:@107",
        "hyperliquid:perp:xyz:TSLA",
    ] {
        let market: Market = id.parse().unwrap();
        assert_eq!(market.to_string(), id);
    }
}

#[test]
fn rejects_malformed_market_ids() {
    let cases = [
        ("", MarketIdError::Malformed),
        ("binance:perp", MarketIdError::Malformed),
        ("binance:perp:", MarketIdError::Malformed),
        (
            "kraken:perp:BTCUSD",
            MarketIdError::UnknownVenue("kraken".into()),
        ),
        (
            "binance:option:BTC",
            MarketIdError::UnknownMarketType("option".into()),
        ),
        (
            "Binance:perp:BTCUSDT",
            MarketIdError::UnknownVenue("Binance".into()),
        ),
        ("binance:perp:BTC.USDT", MarketIdError::InvalidNativeName),
        ("binance:perp:BTC USDT", MarketIdError::InvalidNativeName),
        ("binance:perp:BTC*", MarketIdError::InvalidNativeName),
        ("binance:perp:BTC>", MarketIdError::InvalidNativeName),
    ];
    for (id, expected) in cases {
        assert_eq!(id.parse::<Market>(), Err(expected), "input {id:?}");
    }
}

#[test]
fn market_spec_parses_market_equals_tick() {
    let spec: MarketSpec = "binance:perp:BTCUSDT=0.1".parse().unwrap();

    assert_eq!(spec.market.to_string(), "binance:perp:BTCUSDT");
    assert_eq!(spec.tick.get(), "0.1".parse::<Decimal>().unwrap());
    assert!("binance:perp:BTCUSDT".parse::<MarketSpec>().is_err());
    assert!("binance:perp:BTCUSDT=0".parse::<MarketSpec>().is_err());
}
