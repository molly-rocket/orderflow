# Replay fixtures

JSON Lines, one Venue WebSocket text frame per line: `{"recv_us": <local receive µs>, "text": "<raw frame>"}`. Load with `venue_sim::Fixture::load`; record with `venue-sim capture`.

| File | Market | Source | Contents |
|---|---|---|---|
| `binance-perp-btcusdt-aggtrade.jsonl` | `binance:perp:BTCUSDT` | live capture 2026-11-09, `wss://fstream.binance.com/market/stream?streams=btcusdt@aggTrade`, trimmed to the first 20 s | 6,400 aggTrade frames, aggregate ids contiguous. Engine time `T` 1794179725753 → 1794179745842 ms. Taker buy volume 1074.077, Taker sell volume 293.208 (summed independently with Python `decimal`). |

Re-capture:

```sh
cargo run -p venue-sim -- capture --url "wss://fstream.binance.com/market/stream?streams=btcusdt@aggTrade" --seconds 20 --out fixtures/binance-perp-btcusdt-aggtrade.jsonl
```

If you re-capture, update the expected totals in this table and in `crates/e2e`. Hand-written edge-case fixtures are built in code with `venue_sim::binance::AggTrades`.
