use std::fmt::Display;
use std::future::Future;
use std::io::IsTerminal;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    pub min: Duration,
    pub max: Duration,
}

impl Backoff {
    pub const fn new(min: Duration, max: Duration) -> Self {
        Self { min, max }
    }
}

pub async fn retry<T, E, F, Fut>(
    backoff: Backoff,
    shutdown: &CancellationToken,
    what: &str,
    mut op: F,
) -> Option<T>
where
    E: Display,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut attempts = Retry::new(backoff, shutdown, what);
    loop {
        match op().await {
            Ok(value) => {
                attempts.succeeded();
                return Some(value);
            }
            Err(error) => {
                if !attempts.failed(&error).await {
                    return None;
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct Retry<'a> {
    backoff: Backoff,
    delay: Duration,
    failures: u32,
    shutdown: &'a CancellationToken,
    what: &'a str,
}

impl<'a> Retry<'a> {
    pub fn new(backoff: Backoff, shutdown: &'a CancellationToken, what: &'a str) -> Self {
        Self {
            backoff,
            delay: backoff.min,
            failures: 0,
            shutdown,
            what,
        }
    }

    pub async fn failed(&mut self, error: &impl Display) -> bool {
        self.failures += 1;
        tracing::warn!(
            error = %format!("{error:#}"),
            failures = self.failures,
            delay = ?self.delay,
            "{} failed; retrying",
            self.what
        );
        tokio::select! {
            () = self.shutdown.cancelled() => return false,
            () = tokio::time::sleep(self.delay) => {}
        }
        self.delay = (self.delay * 2).min(self.backoff.max);
        true
    }

    pub fn succeeded(&self) {
        if self.failures > 0 {
            tracing::info!(
                failures = self.failures,
                "{} succeeded after retrying",
                self.what
            );
        }
    }
}

pub fn init_tracing(default_filter: &str) {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| default_filter.into()),
        )
        .with_ansi(std::io::stdout().is_terminal())
        .init();
}

pub fn shutdown_on_signal() -> CancellationToken {
    let shutdown = CancellationToken::new();
    tokio::spawn({
        let shutdown = shutdown.clone();
        async move {
            signal().await;
            tracing::info!("shutting down");
            shutdown.cancel();
        }
    });
    shutdown
}

#[cfg(unix)]
async fn signal() {
    use tokio::signal::unix::{SignalKind, signal};
    match signal(SignalKind::terminate()) {
        Ok(mut terminate) => {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = terminate.recv() => {}
            }
        }
        Err(error) => {
            tracing::warn!(%error, "cannot listen for SIGTERM; Ctrl-C only");
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[cfg(not(unix))]
async fn signal() {
    let _ = tokio::signal::ctrl_c().await;
}
