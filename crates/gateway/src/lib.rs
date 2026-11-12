mod book;
mod bubbles;
mod cvd;
mod dto;
mod http;
mod live;
mod params;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::routing::get;
use orderflow_bus::Bus;
use orderflow_contracts::subjects::Namespace;
pub use orderflow_domain::MarketSpec;
use orderflow_domain::{Decimal, Market, Timestamp};
use orderflow_store::Store;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

pub use dto::{
    BarDto, BookDto, BookMessage, BubbleDto, BubblesResponse, ErrorDto, FootprintResponse, GapDto,
    GroupingDto, HeatmapFrameDto, ImbalanceDto, ImbalanceFlag, LadderRowDto, LargeBubbleDto,
    LiveMessage, MarketDto, RowDto, StackDto,
};

#[derive(Debug, Clone)]
pub struct Config {
    pub namespace: Namespace,
    pub markets: Vec<MarketSpec>,
    pub web_dir: Option<PathBuf>,
}

type ThresholdKey = (Market, Decimal, Timestamp, u64);

#[derive(Debug)]
struct AppState {
    store: Store,
    bus: Bus,
    namespace: Namespace,
    markets: BTreeMap<Market, MarketSpec>,
    thresholds: Mutex<HashMap<ThresholdKey, Option<Decimal>>>,
    books: HashMap<Market, book::MarketBooks>,
    followers: CancellationToken,
}

impl Drop for AppState {
    fn drop(&mut self) {
        self.followers.cancel();
    }
}

fn now() -> Timestamp {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp::from_micros(i64::try_from(since_epoch.as_micros()).unwrap_or(i64::MAX))
}

pub fn router(config: &Config, store: Store, bus: Bus) -> Router {
    let followers = CancellationToken::new();
    let books = book::start(&bus, &config.namespace, &config.markets, &followers);
    let state = Arc::new(AppState {
        store,
        bus,
        namespace: config.namespace.clone(),
        markets: config
            .markets
            .iter()
            .map(|spec| (spec.market.clone(), spec.clone()))
            .collect(),
        thresholds: Mutex::new(HashMap::new()),
        books,
        followers,
    });
    let api = Router::new()
        .route("/api/v1/markets", get(http::markets))
        .route("/api/v1/footprint", get(http::footprint))
        .route("/api/v1/gaps", get(http::gaps))
        .route("/api/v1/bubbles", get(bubbles::bubbles))
        .route("/api/v1/live", get(live::upgrade))
        .route("/api/v1/book", get(book::upgrade))
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
