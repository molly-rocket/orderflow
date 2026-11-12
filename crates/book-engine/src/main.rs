use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use orderflow_book_engine::{Config, binance, run};
use orderflow_bus::Bus;
use orderflow_contracts::subjects::Namespace;
use orderflow_domain::MarketSpec;
use orderflow_runtime::Backoff;

#[derive(Debug, Parser)]
#[command(about = "Hold the live order book per Market and publish DOM states and Heatmap frames")]
struct Args {
    #[arg(long, env = "NATS_URL", default_value = "nats://127.0.0.1:4222")]
    nats_url: String,
    #[arg(
        long,
        env = "MARKETS",
        value_delimiter = ',',
        default_value = "binance:perp:BTCUSDT=0.1"
    )]
    markets: Vec<MarketSpec>,
    #[arg(long, env = "BINANCE_WS_URL", default_value = binance::DEFAULT_WS_URL)]
    binance_ws_url: String,
    #[arg(long, env = "BINANCE_REST_URL", default_value = binance::DEFAULT_REST_URL)]
    binance_rest_url: String,
    #[arg(long, env = "STALL_TIMEOUT_SECS", default_value_t = 30)]
    stall_timeout_secs: u64,
    #[arg(long, env = "NAMESPACE", default_value = "of")]
    namespace: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    orderflow_runtime::init_tracing("info");
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("a rustls crypto provider was already installed"))?;

    let args = Args::parse();
    let bus = Bus::connect(&args.nats_url)
        .await
        .with_context(|| format!("connecting to NATS at {}", args.nats_url))?;
    let config = Config {
        namespace: Namespace::new(&args.namespace)?,
        markets: args.markets,
        ws_url: args.binance_ws_url,
        rest_url: args.binance_rest_url,
        stall_timeout: Duration::from_secs(args.stall_timeout_secs),
        snapshot_timeout: Duration::from_secs(10),
        retry: Backoff::new(Duration::from_millis(250), Duration::from_secs(30)),
        max_buffer: 10_000,
    };
    let shutdown = orderflow_runtime::shutdown_on_signal();
    run(&config, &bus, shutdown).await
}
