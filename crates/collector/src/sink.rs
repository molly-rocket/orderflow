use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use orderflow_bus::{Bus, PublishOutcome};
use orderflow_contracts::subjects::Namespace;

pub use orderflow_contracts::record::Record;

pub trait Sink {
    fn publish(&mut self, record: &Record) -> impl Future<Output = anyhow::Result<()>> + Send;
}

#[derive(Debug, Clone, Default)]
pub struct MemorySink {
    records: Arc<Mutex<Vec<Record>>>,
}

impl MemorySink {
    pub fn records(&self) -> Vec<Record> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Sink for MemorySink {
    async fn publish(&mut self, record: &Record) -> anyhow::Result<()> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record.clone());
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct NatsSink {
    bus: Bus,
    namespace: Namespace,
}

impl NatsSink {
    pub fn new(bus: Bus, namespace: Namespace) -> Self {
        Self { bus, namespace }
    }
}

impl Sink for NatsSink {
    async fn publish(&mut self, record: &Record) -> anyhow::Result<()> {
        let published = self.bus.publish_record(&self.namespace, record).await?;
        if published.is_some_and(|p| p.outcome == PublishOutcome::Duplicate) {
            tracing::debug!(?record, "duplicate publish dropped by JetStream");
        }
        Ok(())
    }
}
