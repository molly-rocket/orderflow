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
cargo xtask live up     # builds the services image, starts collector, writer, gateway
                        #  http://127.0.0.1:8080
cargo xtask verify-live # compare what was recorded with Binance's own trade history
```

The live services run as containers (compose profile `live`) with `restart: unless-stopped`, so they survive crashes and reboots of Docker. Their logs are kept and rotated by Docker (10 × 20 MB per service):

| Command | Does |
|---|---|
| `cargo xtask live logs [collector\|writer\|gateway]` | Last 200 log lines, with timestamps |
| `cargo xtask live status` | Container state and restarts |
| `cargo xtask live down` | Stops the services; NATS, ClickHouse and their data stay |

Restarting the services is safe. The collector records the downtime as a Gap from its checkpoint, and the writer resumes exactly where ClickHouse left off.

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
| `GET /api/v1/footprint?market&from_us&to_us[&timeframe_secs=60&grouping=1&imbalance_ratio=3&min_stack=3]` | Footprint bars (rows with Imbalance flags, stacks, delta, total, `incomplete`) and Gaps |
| `GET /api/v1/gaps?market&from_us&to_us` | Gaps |
| `WS /api/v1/live?market[&timeframe_secs&grouping&imbalance_ratio&min_stack&since_us]` | `{"type":"bar","bar":…,"closed":bool}` and `{"type":"gap","gap":…}` |