use std::collections::HashMap;
use std::fmt::Write as _;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use async_nats::jetstream::kv::Store;
use orderflow_domain::{GapReason, Market, Timestamp};
use serde::{Deserialize, Serialize};

pub const BUCKET_PURPOSE: &str = "collector";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub last_trade_id: u64,
    pub last_engine_time_us: i64,
    pub open_gap: Option<OpenGap>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenGap {
    pub gap_id: String,
    pub start_us: i64,
    pub reason: StoredGapReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoredGapReason {
    Disconnect,
    SequenceBreak,
    BookResync,
    RecorderDowntime,
    StorageOutage,
}

impl From<GapReason> for StoredGapReason {
    fn from(reason: GapReason) -> Self {
        match reason {
            GapReason::Disconnect => Self::Disconnect,
            GapReason::SequenceBreak => Self::SequenceBreak,
            GapReason::BookResync => Self::BookResync,
            GapReason::RecorderDowntime => Self::RecorderDowntime,
            GapReason::StorageOutage => Self::StorageOutage,
        }
    }
}

impl From<StoredGapReason> for GapReason {
    fn from(reason: StoredGapReason) -> Self {
        match reason {
            StoredGapReason::Disconnect => Self::Disconnect,
            StoredGapReason::SequenceBreak => Self::SequenceBreak,
            StoredGapReason::BookResync => Self::BookResync,
            StoredGapReason::RecorderDowntime => Self::RecorderDowntime,
            StoredGapReason::StorageOutage => Self::StorageOutage,
        }
    }
}

impl OpenGap {
    pub fn start(&self) -> Timestamp {
        Timestamp::from_micros(self.start_us)
    }
}

pub trait Checkpoints {
    fn load(
        &self,
        market: &Market,
    ) -> impl Future<Output = anyhow::Result<Option<Checkpoint>>> + Send;
    fn save(
        &self,
        market: &Market,
        checkpoint: &Checkpoint,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
}

#[derive(Debug, Clone, Default)]
pub struct MemoryCheckpoints {
    inner: Arc<Mutex<HashMap<Market, Checkpoint>>>,
}

impl MemoryCheckpoints {
    pub fn get(&self, market: &Market) -> Option<Checkpoint> {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(market)
            .cloned()
    }
}

impl Checkpoints for MemoryCheckpoints {
    async fn load(&self, market: &Market) -> anyhow::Result<Option<Checkpoint>> {
        Ok(self
            .inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(market)
            .cloned())
    }

    async fn save(&self, market: &Market, checkpoint: &Checkpoint) -> anyhow::Result<()> {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(market.clone(), checkpoint.clone());
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct KvCheckpoints {
    store: Store,
}

impl KvCheckpoints {
    pub fn new(store: Store) -> Self {
        Self { store }
    }
}

impl Checkpoints for KvCheckpoints {
    async fn load(&self, market: &Market) -> anyhow::Result<Option<Checkpoint>> {
        match self.store.get(kv_key(market)).await? {
            Some(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            None => Ok(None),
        }
    }

    async fn save(&self, market: &Market, checkpoint: &Checkpoint) -> anyhow::Result<()> {
        self.store
            .put(kv_key(market), serde_json::to_vec(checkpoint)?.into())
            .await?;
        Ok(())
    }
}

fn kv_key(market: &Market) -> String {
    let id = market.to_string();
    let mut key = String::with_capacity(id.len() + 8);
    for byte in id.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'/' | b'_' | b'.') {
            key.push(char::from(byte));
        } else {
            let _ = write!(key, "={byte:02X}");
        }
    }
    key
}

#[cfg(test)]
mod tests {
    use super::kv_key;

    #[test]
    fn kv_keys_escape_characters_nats_does_not_allow() {
        let key = |id: &str| kv_key(&id.parse().unwrap());
        assert_eq!(key("binance:perp:BTCUSDT"), "binance=3Aperp=3ABTCUSDT");
        assert_eq!(key("hyperliquid:spot:@107"), "hyperliquid=3Aspot=3A=40107");
        assert_ne!(
            key("hyperliquid:perp:xyz:TSLA"),
            key("hyperliquid:perp:xyz=3ATSLA")
        );
    }
}
