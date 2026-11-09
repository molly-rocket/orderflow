use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::Message;
use venue_sim::{FakeVenue, Fixture, Frame, Pacing, Script};

#[derive(Debug, Parser)]
#[command(about = "Capture and replay Venue WebSocket sessions")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Capture {
        #[arg(long)]
        url: String,
        #[arg(long, default_value_t = 120)]
        seconds: u64,
        #[arg(long)]
        max_frames: Option<usize>,
        #[arg(long)]
        out: PathBuf,
    },
    Serve {
        #[arg(long)]
        fixture: PathBuf,
        #[arg(long, default_value = "127.0.0.1:9443")]
        addr: SocketAddr,
        #[arg(long, default_value_t = 1.0)]
        speed: f64,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("a rustls crypto provider was already installed"))?;

    match Cli::parse().command {
        Command::Capture {
            url,
            seconds,
            max_frames,
            out,
        } => capture(&url, Duration::from_secs(seconds), max_frames, &out).await,
        Command::Serve {
            fixture,
            addr,
            speed,
        } => serve(&fixture, addr, speed).await,
    }
}

async fn capture(
    url: &str,
    duration: Duration,
    max_frames: Option<usize>,
    out: &PathBuf,
) -> Result<()> {
    let (mut ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .with_context(|| format!("connecting to {url}"))?;
    let deadline = tokio::time::Instant::now() + duration;
    let limit = max_frames.unwrap_or(usize::MAX);
    let mut frames = Vec::new();

    while frames.len() < limit {
        let next = tokio::time::timeout_at(deadline, ws.next()).await;
        match next {
            Err(_) => break, // duration elapsed
            Ok(None) => anyhow::bail!("venue closed the stream after {} frames", frames.len()),
            Ok(Some(message)) => {
                if let Message::Text(text) = message? {
                    frames.push(Frame {
                        recv_us: now_us(),
                        text: text.to_string(),
                    });
                }
            }
        }
    }

    let fixture = Fixture::new(frames);
    let file = std::fs::File::create(out).with_context(|| format!("creating {}", out.display()))?;
    fixture.write_jsonl(std::io::BufWriter::new(file))?;
    println!("captured {} frames into {}", fixture.len(), out.display());
    Ok(())
}

async fn serve(path: &PathBuf, addr: SocketAddr, speed: f64) -> Result<()> {
    let fixture = Fixture::load(path).with_context(|| format!("loading {}", path.display()))?;
    let pacing = if speed > 0.0 {
        Pacing::Realtime { speed }
    } else {
        Pacing::AsFastAsPossible
    };
    let venue = FakeVenue::bind(Script::new(fixture).with_pacing(pacing), addr).await?;
    println!("serving {} on {}", path.display(), venue.url());
    tokio::signal::ctrl_c().await?;
    Ok(())
}

fn now_us() -> i64 {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(since_epoch.as_micros()).unwrap_or(i64::MAX)
}
