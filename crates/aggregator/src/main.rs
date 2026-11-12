use std::time::Duration;

use anyhow::{Context, bail};
use clap::Parser;
use orderflow_aggregator::{Config, run};
use orderflow_bus::Bus;
use orderflow_contracts::subjects::Namespace;
use orderflow_domain::{BubbleGrouping, Market, MarketType, Venue};
use orderflow_runtime::Backoff;

#[derive(Debug, Parser)]
#[command(about = "Derive Bubbles from recorded Trades")]
struct Args {
    #[arg(long, env = "NATS_URL", default_value = "nats://127.0.0.1:4222")]
    nats_url: String,
    #[arg(
        long,
        env = "MARKETS",
        value_delimiter = ',',
        default_value = "binance:perp:BTCUSDT"
    )]
    markets: Vec<Market>,
    #[arg(long, env = "NAMESPACE", default_value = "of")]
    namespace: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    orderflow_runtime::init_tracing("info");
    let args = Args::parse();
    if let Some(market) = args
        .markets
        .iter()
        .find(|m| m.venue() != Venue::Binance || m.market_type() != MarketType::Perp)
    {
        bail!(
            "market {market} is not a Binance perp market; Bubble grouping is defined for Binance USD-M only"
        );
    }
    let bus = Bus::connect(&args.nats_url)
        .await
        .with_context(|| format!("connecting to NATS at {}", args.nats_url))?;
    let config = Config {
        namespace: Namespace::new(&args.namespace)?,
        markets: args.markets,
        grouping: BubbleGrouping::BINANCE_PERP,
        retry: Backoff::new(Duration::from_millis(100), Duration::from_secs(5)),
    };
    let shutdown = orderflow_runtime::shutdown_on_signal();
    run(&config, &bus, shutdown).await
}
