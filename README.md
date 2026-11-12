# orderflow

A crypto orderflow web app that records Binance and Hyperliquid order flow and shows it as Footprint, Bubbles, CVD, an order book Heatmap, a DOM and filters. **Built so far:** Binance USD-M perp Trades → Footprint with Imbalances, Stacked Imbalances and Gaps, as history and live.



## Requirements

- Rust (stable, edition 2024)
- Docker with Compose v2
- Node 20+ with pnpm (for `web/`)
- No `protoc`: Protobuf is compiled in Rust by `protox`.

## Run it on live data

```sh
cargo xtask up          # NATS (JetStream) + ClickHouse, waits until healthy
cargo xtask live up     # builds the services image, starts collector, aggregator, book-engine, writer, gateway
                        #  http://127.0.0.1:8080
cargo xtask verify-live # compare what was recorded with Binance's own trade history
```

The live services run as containers (compose profile `live`) with `restart: unless-stopped`, so they survive crashes and reboots of Docker. Their logs are kept and rotated by Docker (10 × 20 MB per service):

| Command | Does |
|---|---|
| `cargo xtask live logs [collector\|aggregator\|book-engine\|writer\|gateway]` | Last 200 log lines, with timestamps |
| `cargo xtask live status` | Container state and restarts |
| `cargo xtask live down` | Stops the services; NATS, ClickHouse and their data stay |

Restarting the services is safe. The collector records the downtime as a Gap from its checkpoint, and the writer resumes exactly where ClickHouse left off.

### Using the chart

- **Price as**: Footprint, Candles, Line or Hidden, switched without a reload. Rows (Price Grouping) default to Auto, about 30 rows per bar.
- **Layers**: Bubbles, Heatmap, Book (the order book ladder), CVD and Price (the live price line and tag) each toggle on their own.
- **Order book**: the ladder right of the price axis shows resting bids (green) and asks (red) on the chart's price scale, with the best bid, ask and spread in the top bar. Binance publishes only about ±0.2 % of depth around the price; outside that *known range* the ladder and the Heatmap are hatched, never drawn as empty. The book is live only: when it is resyncing or disconnected the ladder says so and shows no rows.
- **Heatmap**: resting liquidity over time, from the gateway's last 2 hours of frames (one per second); off by default.
- **Bubbles** sit at their VWAP, area by notional, colored by Taker side; Large Bubbles have a ring and a USD label. Hover any Bubble, Footprint cell, bar total or CVD point for details.
- **CVD** resets at 00:00 UTC and turns dashed and labelled once a Gap makes it incomplete for the day.
- **Settings** (sliders icon): Imbalance ratio and stack length, Large Bubble percentile and USD floor, minimum Bubble notional, Taker side.
- **Navigate**: drag to pan, wheel to zoom time, Ctrl/⌘ + wheel or drag the price axis to zoom price, double-click to return to live.
- Everything you choose is in the URL (shareable) and remembered in your browser.

### Run from source (development)

```sh
cargo xtask web                     # builds web/dist
cargo run -p orderflow-collector    # records binance:perp:BTCUSDT from the live Venue
cargo run -p orderflow-writer       # stream → ClickHouse
WEB_DIR=web/dist cargo run -p orderflow-gateway   # http://127.0.0.1:8080
```

Stop the live containers first (`cargo xtask live down`). Two writers on the same namespace would both write every Trade.

Configuration is by flags or environment variables (`--help` on each binary):

| Variable | Used by | Default |
|---|---|---|
| `NATS_URL` | all | `nats://127.0.0.1:4222` |
| `NAMESPACE` | all | `of` (ADR-0002) |
| `MARKETS` | collector: `binance:perp:BTCUSDT,...`; gateway: `binance:perp:BTCUSDT=0.1,...` (with tick size) | BTCUSDT |
| `VENUE_URL`, `STALL_TIMEOUT_SECS` | collector | `wss://fstream.binance.com/market`, 30 |
| `CLICKHOUSE_URL`, `CLICKHOUSE_DATABASE`, `CLICKHOUSE_USER`, `CLICKHOUSE_PASSWORD` | writer, gateway | local stack |
| `LISTEN`, `WEB_DIR` | gateway | `127.0.0.1:8080`, none |

For web development, run `pnpm --dir web dev` (port 5173). It proxies `/api` to the gateway.

## HTTP API

| Route | Returns |
|---|---|
| `GET /api/v1/markets` | Served Markets: tick size, Recording Horizon, burst-grouped flag |
| `GET /api/v1/footprint?market&from_us&to_us[&timeframe_secs=60&grouping=1&imbalance_ratio=3&min_stack=3]` | Footprint bars (rows with Imbalance flags, stacks, delta, total, exact `open`/`high`/`low`/`close`, `cvd`, `cvd_complete`, `incomplete`) and Gaps |
| `GET /api/v1/bubbles?market&from_us&to_us[&percentile=99&usd_floor&min_notional&side=both]` | Bubbles (side, VWAP, size, notional, Trades, price range, `large`), the Large Bubble threshold and Gaps. At most 3 days and 20,000 Bubbles per request (the largest, with `truncated`) |
| `GET /api/v1/gaps?market&from_us&to_us` | Gaps |
| `WS /api/v1/book?market[&grouping=1&timeframe_secs=60&heatmap_since_us]` | Live order book: `{"type":"book",…}` with the DOM ladder at the Price Grouping (status `live`/`resyncing`/`disconnected`, best bid and ask, the known range, rows), and with `heatmap_since_us` a `heatmap_history` burst followed by one `heatmap` frame per second. Live only: the gateway keeps the last 2 h of frames in memory |
| `WS /api/v1/live?market[&timeframe_secs&grouping&imbalance_ratio&min_stack&percentile&usd_floor&min_notional&side&since_us]` | `{"type":"bar",…}`, `{"type":"gap",…}` and `{"type":"bubble","bubble":…}` |