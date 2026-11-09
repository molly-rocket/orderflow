use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use orderflow_bus::Bus;
use orderflow_contracts::subjects::Namespace;
use orderflow_gateway::{Config, MarketSpec, router, serve};
use orderflow_store::{Store, StoreConfig};

#[derive(Debug, Parser)]
#[command(about = "Serve orderflow history and live data to browsers")]
struct Args {
    #[arg(long, env = "LISTEN", default_value = "127.0.0.1:8080")]
    listen: SocketAddr,
    #[arg(long, env = "NATS_URL", default_value = "nats://127.0.0.1:4222")]
    nats_url: String,
    #[arg(long, env = "CLICKHOUSE_URL", default_value = "http://127.0.0.1:8123")]
    clickhouse_url: String,
    #[arg(long, env = "CLICKHOUSE_DATABASE", default_value = "orderflow")]
    clickhouse_database: String,
    #[arg(long, env = "CLICKHOUSE_USER", default_value = "orderflow")]
    clickhouse_user: String,
    #[arg(long, env = "CLICKHOUSE_PASSWORD", default_value = "orderflow")]
    clickhouse_password: String,
    #[arg(
        long,
        env = "MARKETS",
        value_delimiter = ',',
        default_value = "binance:perp:BTCUSDT=0.1"
    )]
    markets: Vec<MarketSpec>,
    #[arg(long, env = "NAMESPACE", default_value = "of")]
    namespace: String,
    #[arg(long, env = "WEB_DIR")]
    web_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    orderflow_runtime::init_tracing("info,tower_http=info");
    let args = Args::parse();
    let bus = Bus::connect(&args.nats_url).await?;
    let store = Store::connect(&StoreConfig {
        url: args.clickhouse_url,
        database: args.clickhouse_database,
        user: args.clickhouse_user,
        password: args.clickhouse_password,
    })
    .await?;
    let config = Config {
        namespace: Namespace::new(&args.namespace)?,
        markets: args.markets,
        web_dir: args.web_dir,
    };

    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!(addr = %args.listen, "listening");
    let shutdown = orderflow_runtime::shutdown_on_signal();
    serve(listener, router(&config, store, bus), shutdown).await?;
    Ok(())
}
