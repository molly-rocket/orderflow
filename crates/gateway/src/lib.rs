mod dto;
mod http;
mod live;
mod params;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use orderflow_bus::Bus;
use orderflow_contracts::subjects::Namespace;
use orderflow_domain::{Market, TickSize};
use orderflow_store::Store;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

pub use dto::{
    BarDto, ErrorDto, FootprintResponse, GapDto, GroupingDto, ImbalanceDto, ImbalanceFlag,
    LiveMessage, MarketDto, RowDto, StackDto,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketSpec {
    pub market: Market,
    pub tick: TickSize,
}

impl FromStr for MarketSpec {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        let (market, tick) = s
            .rsplit_once('=')
            .ok_or_else(|| anyhow::anyhow!("expected `market=tick`, got `{s}`"))?;
        Ok(Self {
            market: market.parse()?,
            tick: TickSize::new(tick.parse()?)?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub namespace: Namespace,
    pub markets: Vec<MarketSpec>,
    pub web_dir: Option<PathBuf>,
}

#[derive(Debug)]
struct AppState {
    store: Store,
    bus: Bus,
    namespace: Namespace,
    markets: BTreeMap<Market, MarketSpec>,
}

pub fn router(config: &Config, store: Store, bus: Bus) -> Router {
    let state = Arc::new(AppState {
        store,
        bus,
        namespace: config.namespace.clone(),
        markets: config
            .markets
            .iter()
            .map(|spec| (spec.market.clone(), spec.clone()))
            .collect(),
    });
    let api = Router::new()
        .route("/api/v1/markets", get(http::markets))
        .route("/api/v1/footprint", get(http::footprint))
        .route("/api/v1/gaps", get(http::gaps))
        .route("/api/v1/live", get(live::upgrade))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state);
    let app = match &config.web_dir {
        Some(dir) => api.fallback_service(
            ServeDir::new(dir).not_found_service(ServeFile::new(dir.join("index.html"))),
        ),
        None => api,
    };
    app.layer(TraceLayer::new_for_http())
}

pub async fn serve(
    listener: TcpListener,
    app: Router,
    shutdown: CancellationToken,
) -> std::io::Result<()> {
    axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await
}
