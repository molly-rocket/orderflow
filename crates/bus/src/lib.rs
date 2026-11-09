use std::time::Duration;

use async_nats::HeaderMap;
use async_nats::jetstream::{self, kv, stream};
use orderflow_contracts::record::{Record, RecordError};
use orderflow_contracts::subjects::{self, Namespace};


pub const RECORD_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);

pub const DEDUPE_WINDOW: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamSpec {
    pub name: String,
    pub subjects: Vec<String>,
}

impl StreamSpec {
    pub fn record(namespace: &subjects::Namespace) -> Self {
        Self {
            name: namespace.record_stream(),
            subjects: namespace.record_stream_subjects(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishOutcome {
    Stored,
    Duplicate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Published {
    pub outcome: PublishOutcome,
    pub sequence: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("message has no `{}` header", subjects::TYPE_HEADER)]
    MissingType,
    #[error("expected `{expected}`, message is `{actual}`")]
    UnexpectedType { expected: String, actual: String },
    #[error("payload does not decode: {0}")]
    Payload(#[from] prost::DecodeError),
    #[error(transparent)]
    Record(#[from] RecordError),
}

#[derive(Debug, Clone)]
pub struct Bus {
    client: async_nats::Client,
    jetstream: jetstream::Context,
}

impl Bus {
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let client = async_nats::ConnectOptions::new()
            .name(env!("CARGO_PKG_NAME"))
            .retry_on_initial_connect()
            .connect(url)
            .await?;
        Ok(Self::from_client(client))
    }

    pub fn from_client(client: async_nats::Client) -> Self {
        let jetstream = jetstream::new(client.clone());
        Self { client, jetstream }
    }

    pub fn client(&self) -> &async_nats::Client {
        &self.client
    }

    pub fn jetstream(&self) -> &jetstream::Context {
        &self.jetstream
    }

    pub async fn ensure_stream(&self, spec: &StreamSpec) -> anyhow::Result<stream::Stream> {
        let config = stream::Config {
            name: spec.name.clone(),
            subjects: spec.subjects.clone(),
            storage: stream::StorageType::File,
            retention: stream::RetentionPolicy::Limits,
            max_age: RECORD_RETENTION,
            duplicate_window: DEDUPE_WINDOW,
            ..Default::default()
        };
        Ok(self.jetstream.get_or_create_stream(config).await?)
    }

    pub async fn delete_stream(&self, name: &str) -> anyhow::Result<()> {
        self.jetstream.delete_stream(name).await?;
        Ok(())
    }

    pub async fn key_value(&self, bucket: &str) -> anyhow::Result<kv::Store> {
        if let Ok(store) = self.jetstream.get_key_value(bucket).await {
            return Ok(store);
        }
        Ok(self
            .jetstream
            .create_key_value(kv::Config {
                bucket: bucket.to_owned(),
                history: 1,
                storage: stream::StorageType::File,
                ..Default::default()
            })
            .await?)
    }

    pub async fn publish<M>(
        &self,
        subject: &str,
        msg_id: Option<&str>,
        message: &M,
    ) -> anyhow::Result<Published>
    where
        M: prost::Message + prost::Name,
    {
        let mut headers = HeaderMap::new();
        headers.insert(subjects::TYPE_HEADER, M::full_name().as_str());
        if let Some(id) = msg_id {
            headers.insert(async_nats::header::NATS_MESSAGE_ID, id);
        }
        let ack = self
            .jetstream
            .publish_with_headers(subject.to_owned(), headers, message.encode_to_vec().into())
            .await?
            .await?;
        Ok(Published {
            outcome: if ack.duplicate {
                PublishOutcome::Duplicate
            } else {
                PublishOutcome::Stored
            },
            sequence: ack.sequence,
        })
    }
}

impl Bus {
    pub async fn publish_record(
        &self,
        namespace: &Namespace,
        record: &Record,
    ) -> anyhow::Result<Option<Published>> {
        let Some(msg_id) = record.dedupe_id() else {
            return Ok(None);
        };
        let subject = match record {
            Record::Trades { market, .. } => namespace.trades(market),
            Record::GapOpened { market, .. } | Record::GapClosed { market, .. } => {
                namespace.gaps(market)
            }
        };
        let encoded = record.encode()?;
        let mut headers = HeaderMap::new();
        headers.insert(subjects::TYPE_HEADER, encoded.type_name.as_str());
        headers.insert(async_nats::header::NATS_MESSAGE_ID, msg_id.as_str());
        let ack = self
            .jetstream
            .publish_with_headers(subject, headers, encoded.payload.into())
            .await?
            .await?;
        Ok(Some(Published {
            outcome: if ack.duplicate {
                PublishOutcome::Duplicate
            } else {
                PublishOutcome::Stored
            },
            sequence: ack.sequence,
        }))
    }
}

pub fn decode_record(headers: Option<&HeaderMap>, payload: &[u8]) -> Result<Record, DecodeError> {
    let type_name = type_of(headers).ok_or(DecodeError::MissingType)?;
    Ok(Record::decode(type_name, payload)?)
}

pub fn type_of(headers: Option<&HeaderMap>) -> Option<&str> {
    headers?
        .get(subjects::TYPE_HEADER)
        .map(async_nats::HeaderValue::as_str)
}

pub fn decode<M>(headers: Option<&HeaderMap>, payload: &[u8]) -> Result<M, DecodeError>
where
    M: prost::Message + prost::Name + Default,
{
    let actual = type_of(headers).ok_or(DecodeError::MissingType)?;
    let expected = M::full_name();
    if actual != expected {
        return Err(DecodeError::UnexpectedType {
            expected,
            actual: actual.to_owned(),
        });
    }
    Ok(M::decode(payload)?)
}
