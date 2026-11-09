CREATE TABLE IF NOT EXISTS trades
(
    market          LowCardinality(String),
    venue_trade_id  UInt64,
    engine_time_us  Int64,
    price           Decimal128(18),
    size            Decimal128(18),
    taker_side      Enum8('buy' = 1, 'sell' = 2),
    taker_order_id  Nullable(String),
    stream_seq      UInt64
)
ENGINE = MergeTree
ORDER BY (market, engine_time_us, venue_trade_id);


CREATE TABLE IF NOT EXISTS footprint_1m
(
    market        LowCardinality(String),
    bar_start_us  Int64,
    price         Decimal128(18),
    buy_volume    Decimal128(18),
    sell_volume   Decimal128(18),
    trade_count   UInt64
)
ENGINE = SummingMergeTree
ORDER BY (market, bar_start_us, price);

CREATE MATERIALIZED VIEW IF NOT EXISTS footprint_1m_mv TO footprint_1m AS
SELECT
    market,
    intDiv(engine_time_us, 60000000) * 60000000 AS bar_start_us,
    price,
    sumIf(size, taker_side = 'buy')  AS buy_volume,
    sumIf(size, taker_side = 'sell') AS sell_volume,
    count()                          AS trade_count
FROM trades
GROUP BY market, bar_start_us, price;


CREATE TABLE IF NOT EXISTS gap_events
(
    market      LowCardinality(String),
    gap_id      String,
    kind        Enum8('open' = 1, 'close' = 2),
    time_us     Int64,
    reason      Enum8('none' = 0, 'disconnect' = 1, 'sequence_break' = 2, 'book_resync' = 3, 'recorder_downtime' = 4, 'storage_outage' = 5),
    stream_seq  UInt64
)
ENGINE = ReplacingMergeTree
ORDER BY (market, gap_id, kind);
