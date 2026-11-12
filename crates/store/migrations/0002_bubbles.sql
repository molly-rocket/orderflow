CREATE TABLE IF NOT EXISTS bubbles
(
    market          LowCardinality(String),
    first_trade_id  UInt64,
    last_trade_id   UInt64,
    start_us        Int64,
    end_us          Int64,
    taker_side      Enum8('buy' = 1, 'sell' = 2),
    trade_count     UInt32,
    size            Decimal128(18),
    notional        Decimal128(18),
    low             Decimal128(18),
    high            Decimal128(18),
    stream_seq      UInt64
)
ENGINE = ReplacingMergeTree
ORDER BY (market, start_us, first_trade_id);
