use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use orderflow_bus::{Bus, StreamSpec};
use orderflow_collector::checkpoint::{BUCKET_PURPOSE, KvCheckpoints};
use orderflow_collector::sink::NatsSink;
use orderflow_collector::{Config, binance, run};
use orderflow_contracts::subjects::Namespace;
use orderflow_domain::Market;
use orderflow_runtime::Backoff;

#[derive(Debug, Parser)]
#[command(about = "Record Venue Trades and Gaps into NATS")]
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
    #[arg(long, env = "VENUE_URL", default_value = binance::DEFAULT_BASE_URL)]
    venue_url: String,
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
    let namespace = Namespace::new(&args.namespace)?;
    bus.ensure_stream(&StreamSpec::record(&namespace)).await?;
    let checkpoints =
        KvCheckpoints::new(bus.key_value(&namespace.kv_bucket(BUCKET_PURPOSE)).await?);
    let mut sink = NatsSink::new(bus, namespace);

    let config = Config {
        venue_base_url: args.venue_url,
        markets: args.markets,
        stall_timeout: Duration::from_secs(args.stall_timeout_secs),
        reconnect_min: Duration::from_millis(250),
        reconnect_max: Duration::from_secs(30),
        checkpoint_interval: Duration::from_secs(1),
        retry: Backoff::new(Duration::from_millis(100), Duration::from_secs(5)),
    };

    let shutdown = orderflow_runtime::shutdown_on_signal();
    run(&config, &mut sink, &checkpoints, shutdown).await
}
