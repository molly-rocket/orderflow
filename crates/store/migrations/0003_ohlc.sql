CREATE TABLE IF NOT EXISTS ohlc_1m
(
    market        LowCardinality(String),
    bar_start_us  Int64,
    open          AggregateFunction(argMin, Decimal128(18), Tuple(Int64, UInt64)),
    close         AggregateFunction(argMax, Decimal128(18), Tuple(Int64, UInt64)),
    high          SimpleAggregateFunction(max, Decimal128(18)),
    low           SimpleAggregateFunction(min, Decimal128(18))
)
ENGINE = AggregatingMergeTree
ORDER BY (market, bar_start_us);

CREATE MATERIALIZED VIEW IF NOT EXISTS ohlc_1m_mv TO ohlc_1m AS
SELECT
    market,
    intDiv(engine_time_us, 60000000) * 60000000 AS bar_start_us,
    argMinState(price, (engine_time_us, venue_trade_id)) AS open,
    argMaxState(price, (engine_time_us, venue_trade_id)) AS close,
    max(price) AS high,
    min(price) AS low
FROM trades
GROUP BY market, bar_start_us;

INSERT INTO ohlc_1m
SELECT
    market,
    intDiv(engine_time_us, 60000000) * 60000000 AS bar_start_us,
    argMinState(price, (engine_time_us, venue_trade_id)) AS open,
    argMaxState(price, (engine_time_us, venue_trade_id)) AS close,
    max(price) AS high,
    min(price) AS low
FROM trades
GROUP BY market, bar_start_us;
