#![allow(clippy::missing_panics_doc)]

pub use venue_sim::FaultProxy;

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use futures_util::StreamExt;
use orderflow_bus::{Bus, StreamSpec};
use orderflow_collector::checkpoint::{BUCKET_PURPOSE, KvCheckpoints};
use orderflow_collector::sink::NatsSink;
use orderflow_contracts::subjects::Namespace;
use orderflow_domain::BubbleGrouping;
use orderflow_gateway::MarketSpec;
use orderflow_store::{Store, StoreConfig};
use serde_json::Value;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;
use venue_sim::{FakeRest, FakeVenue, Script};

pub use venue_sim::captures;

pub const NATS_URL: &str = "nats://127.0.0.1:4222";

pub const MARKET: &str = "binance:perp:BTCUSDT";

#[derive(Debug)]
pub struct Stack {
    pub venue: FakeVenue,
    pub gateway_url: String,
    namespace: Namespace,
    bus: Bus,
    store: Store,
    storage: FaultProxy,
    http: reqwest::Client,
    shutdown: CancellationToken,
    tasks: Vec<JoinHandle<anyhow::Result<()>>>,
    aggregator: Option<(CancellationToken, JoinHandle<anyhow::Result<()>>)>,
    book_engine: Option<BookEngine>,
}

#[derive(Debug)]
struct BookEngine {
    _venue: FakeVenue,
    _rest: FakeRest,
    shutdown: CancellationToken,
    task: JoinHandle<anyhow::Result<()>>,
}

fn aggregator_config(namespace: &Namespace) -> anyhow::Result<orderflow_aggregator::Config> {
    Ok(orderflow_aggregator::Config {
        namespace: namespace.clone(),
        markets: vec![MARKET.parse()?],
        grouping: BubbleGrouping::BINANCE_PERP,
        retry: orderflow_collector::Backoff::new(
            Duration::from_millis(20),
            Duration::from_millis(200),
        ),
    })
}

fn spawn_aggregator(
    namespace: &Namespace,
    bus: &Bus,
) -> anyhow::Result<(CancellationToken, JoinHandle<anyhow::Result<()>>)> {
    let config = aggregator_config(namespace)?;
    let shutdown = CancellationToken::new();
    let task = tokio::spawn({
        let (bus, shutdown) = (bus.clone(), shutdown.clone());
        async move { orderflow_aggregator::run(&config, &bus, shutdown).await }
    });
    Ok((shutdown, task))
}

impl Stack {
    pub async fn start(script: Script) -> Self {
        Self::try_start(script)
            .await
            .expect("start the e2e stack (is `cargo xtask up` running?)")
    }

    async fn try_start(script: Script) -> anyhow::Result<Self> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() % 1_000_000_000;
        let id = format!(
            "e2e{}_{nonce}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let namespace = Namespace::new(&id)?;

        let bus = Bus::connect(NATS_URL).await?;
        bus.ensure_stream(&StreamSpec::record(&namespace)).await?;
        let store_config = StoreConfig {
            database: id.clone(),
            ..StoreConfig::local()
        };
        let store = Store::connect(&store_config).await?;
        let venue = FakeVenue::start(script).await?;
        let shutdown = CancellationToken::new();
        let mut tasks = Vec::new();

        let collector_config = orderflow_collector::Config {
            venue_base_url: venue.url(),
            markets: vec![MARKET.parse()?],
            stall_timeout: Duration::from_secs(10),
            reconnect_min: Duration::from_millis(20),
            reconnect_max: Duration::from_millis(200),
            checkpoint_interval: Duration::from_millis(100),
            retry: orderflow_collector::Backoff::new(
                Duration::from_millis(20),
                Duration::from_millis(200),
            ),
        };
        let checkpoints =
            KvCheckpoints::new(bus.key_value(&namespace.kv_bucket(BUCKET_PURPOSE)).await?);
        let mut sink = NatsSink::new(bus.clone(), namespace.clone());
        tasks.push(tokio::spawn({
            let shutdown = shutdown.clone();
            async move {
                orderflow_collector::run(&collector_config, &mut sink, &checkpoints, shutdown).await
            }
        }));

        let writer_config = orderflow_writer::Config {
            linger: Duration::from_millis(20),
            ..orderflow_writer::Config::for_namespace(&namespace)
        };
        tasks.push(tokio::spawn({
            let (bus, store, shutdown) = (bus.clone(), store.clone(), shutdown.clone());
            async move { orderflow_writer::run(&writer_config, &bus, &store, shutdown).await }
        }));

        let aggregator = Some(spawn_aggregator(&namespace, &bus)?);

        let gateway_config = orderflow_gateway::Config {
            namespace: namespace.clone(),
            markets: vec![format!("{MARKET}={}", captures::btc_perp::TICK).parse::<MarketSpec>()?],
            web_dir: None,
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let gateway_url = format!("http://{}", listener.local_addr()?);
        let storage = FaultProxy::start(
            store_config
                .url
                .trim_start_matches("http://")
                .trim_end_matches('/')
                .to_owned(),
        )
        .await?;
        let gateway_store = Store::connect(&StoreConfig {
            url: storage.url(),
            ..store_config
        })
        .await?;
        let app = orderflow_gateway::router(&gateway_config, gateway_store, bus.clone());
        tasks.push(tokio::spawn({
            let shutdown = shutdown.clone();
            async move { Ok(orderflow_gateway::serve(listener, app, shutdown).await?) }
        }));

        Ok(Self {
            venue,
            gateway_url,
            namespace,
            bus,
            store,
            storage,
            http: reqwest::Client::new(),
            shutdown,
            tasks,
            aggregator,
            book_engine: None,
        })
    }

    pub async fn start_book(&mut self, depth: Script, snapshots: Vec<(u16, String)>) {
        let venue = FakeVenue::start(depth).await.expect("fake depth venue");
        let rest = FakeRest::start(snapshots).await.expect("fake REST");
        let config = orderflow_book_engine::Config {
            namespace: self.namespace.clone(),
            markets: vec![
                format!("{MARKET}={}", captures::btc_perp::TICK)
                    .parse::<MarketSpec>()
                    .expect("market spec"),
            ],
            ws_url: venue.url(),
            rest_url: rest.url(),
            stall_timeout: Duration::from_secs(10),
            snapshot_timeout: Duration::from_secs(2),
            retry: orderflow_collector::Backoff::new(
                Duration::from_millis(20),
                Duration::from_millis(200),
            ),
            max_buffer: 1000,
        };
        let shutdown = CancellationToken::new();
        let task = tokio::spawn({
            let (bus, shutdown) = (self.bus.clone(), shutdown.clone());
            async move { orderflow_book_engine::run(&config, &bus, shutdown).await }
        });
        self.book_engine = Some(BookEngine {
            _venue: venue,
            _rest: rest,
            shutdown,
            task,
        });
    }

    pub async fn book(&self, query: &str) -> LiveClient {
        let url = format!(
            "{}/api/v1/book?market={MARKET}{query}",
            self.gateway_url.replacen("http", "ws", 1)
        );
        let (ws, _) = tokio_tungstenite::connect_async(url)
            .await
            .expect("book upgrade");
        LiveClient { ws }
    }

    pub async fn restart_aggregator(&mut self) {
        if let Some((shutdown, task)) = self.aggregator.take() {
            shutdown.cancel();
            task.await
                .expect("aggregator task panicked")
                .expect("aggregator failed");
        }
        self.aggregator =
            Some(spawn_aggregator(&self.namespace, &self.bus).expect("restart the aggregator"));
    }

    pub async fn bubbles(&self, from_us: i64, to_us: i64, query: &str) -> Value {
        let (status, body) = self
            .get(&format!(
                "/api/v1/bubbles?market={MARKET}&from_us={from_us}&to_us={to_us}{query}"
            ))
            .await;
        assert_eq!(status, 200, "{body}");
        body
    }

    pub async fn wait_for_bubble_trades(&self, from_us: i64, to_us: i64, trades: u64) -> Value {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let body = self.bubbles(from_us, to_us, "").await;
            let seen: u64 = body["bubbles"]
                .as_array()
                .expect("bubbles array")
                .iter()
                .map(|b| b["trades"].as_u64().expect("trade count"))
                .sum();
            if seen >= trades {
                assert_eq!(seen, trades, "more Bubble Trades than replayed");
                return body;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "Bubbles cover {seen} of {trades} Trades"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub async fn get_response(&self, path_and_query: &str) -> reqwest::Response {
        self.http
            .get(format!("{}{path_and_query}", self.gateway_url))
            .send()
            .await
            .expect("gateway request")
    }

    pub async fn get(&self, path_and_query: &str) -> (u16, Value) {
        let response = self.get_response(path_and_query).await;
        let status = response.status().as_u16();
        (status, response.json().await.expect("gateway returns JSON"))
    }

    pub async fn footprint(&self, from_us: i64, to_us: i64, query: &str) -> Value {
        let (status, body) = self
            .get(&format!(
                "/api/v1/footprint?market={MARKET}&from_us={from_us}&to_us={to_us}{query}"
            ))
            .await;
        assert_eq!(status, 200, "{body}");
        body
    }

    pub async fn wait_for_trades(&self, from_us: i64, to_us: i64, trades: u64) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let body = self.footprint(from_us, to_us, "").await;
            let seen: u64 = body["bars"]
                .as_array()
                .expect("bars array")
                .iter()
                .map(|b| b["trades"].as_u64().expect("trades count"))
                .sum();
            if seen >= trades {
                assert_eq!(seen, trades, "more Trades than replayed");
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "gateway shows {seen} of {trades} Trades"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub async fn live(&self, query: &str) -> LiveClient {
        let url = format!(
            "{}/api/v1/live?market={MARKET}{query}",
            self.gateway_url.replacen("http", "ws", 1)
        );
        let (ws, _) = tokio_tungstenite::connect_async(url)
            .await
            .expect("live upgrade");
        LiveClient { ws }
    }

    pub fn cut_storage(&self) {
        self.storage.cut();
    }

    pub fn restore_storage(&self) {
        self.storage.restore();
    }

    pub async fn stop(self) {
        self.shutdown.cancel();
        if let Some((shutdown, task)) = self.aggregator {
            shutdown.cancel();
            task.await
                .expect("aggregator task panicked")
                .expect("aggregator failed");
        }
        if let Some(engine) = self.book_engine {
            engine.shutdown.cancel();
            engine
                .task
                .await
                .expect("book-engine task panicked")
                .expect("book-engine failed");
        }
        for task in self.tasks {
            task.await
                .expect("service task panicked")
                .expect("service failed");
        }
        let js = self.bus.jetstream();
        js.delete_stream(self.namespace.record_stream())
            .await
            .expect("delete stream");
        js.delete_key_value(self.namespace.kv_bucket(BUCKET_PURPOSE))
            .await
            .expect("delete kv bucket");
        self.store
            .drop_database_for_tests()
            .await
            .expect("drop database");
    }
}

#[derive(Debug)]
pub struct LiveClient {
    ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
}

impl LiveClient {
    pub async fn read_until(&mut self, done: impl Fn(&[Value]) -> bool) -> Vec<Value> {
        let mut seen = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        while !done(&seen) {
            let next = tokio::time::timeout_at(deadline, self.ws.next())
                .await
                .with_context(|| format!("timed out; received {seen:#?}"))
                .unwrap();
            match next {
                Some(Ok(Message::Text(text))) => {
                    seen.push(serde_json::from_str(&text).expect("live messages are JSON"));
                }
                Some(Ok(_)) => {}
                other => panic!("live socket ended: {other:?}"),
            }
        }
        seen
    }
}
