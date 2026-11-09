use clap::Parser;
use orderflow_bus::Bus;
use orderflow_store::{Store, StoreConfig};
use orderflow_writer::{Config, run};

#[derive(Debug, Parser)]
#[command(about = "Write recorded Trades and Gaps to ClickHouse")]
struct Args {
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
    #[arg(long, env = "NAMESPACE", default_value = "of")]
    namespace: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    orderflow_runtime::init_tracing("info");
    let args = Args::parse();
    let bus = Bus::connect(&args.nats_url).await?;
    let store = Store::connect(&StoreConfig {
        url: args.clickhouse_url,
        database: args.clickhouse_database,
        user: args.clickhouse_user,
        password: args.clickhouse_password,
    })
    .await?;

    let shutdown = orderflow_runtime::shutdown_on_signal();
    let namespace = orderflow_contracts::subjects::Namespace::new(&args.namespace)?;
    run(&Config::for_namespace(&namespace), &bus, &store, shutdown).await
}
