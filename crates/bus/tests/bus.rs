use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use orderflow_bus::{Bus, DecodeError, PublishOutcome, StreamSpec, decode};
use orderflow_contracts::v1::{GapClosed, TradeBatch};

const NATS_URL: &str = "nats://127.0.0.1:4222";

fn spec(test: &str) -> StreamSpec {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    StreamSpec {
        name: format!("TEST_{}_{nonce}", test.to_uppercase()),
        subjects: vec![format!("test.{test}.{nonce}.>")],
    }
}

fn batch(market: &str) -> TradeBatch {
    TradeBatch {
        market: market.into(),
        trades: vec![],
    }
}

#[tokio::test]
async fn published_messages_are_stored_with_their_type_and_decoded_back() {
    let bus = Bus::connect(NATS_URL).await.unwrap();
    let spec = spec("roundtrip");
    let stream = bus.ensure_stream(&spec).await.unwrap();
    let subject = spec.subjects[0].replace('>', "trade");

    let published = bus
        .publish(&subject, Some("m-1"), &batch("binance:perp:BTCUSDT"))
        .await
        .unwrap();
    assert_eq!(published.outcome, PublishOutcome::Stored);
    assert_eq!(published.sequence, 1);

    let stored = stream
        .get_last_raw_message_by_subject(&subject)
        .await
        .unwrap();
    let decoded: TradeBatch = decode(Some(&stored.headers), &stored.payload).unwrap();
    assert_eq!(decoded, batch("binance:perp:BTCUSDT"));
    bus.delete_stream(&spec.name).await.unwrap();
}

#[tokio::test]
async fn republishing_the_same_message_id_is_deduplicated() {
    let bus = Bus::connect(NATS_URL).await.unwrap();
    let spec = spec("dedupe");
    bus.ensure_stream(&spec).await.unwrap();
    let subject = spec.subjects[0].replace('>', "trade");

    let first = bus
        .publish(&subject, Some("same"), &batch("a:b:c"))
        .await
        .unwrap();
    let again = bus
        .publish(&subject, Some("same"), &batch("a:b:c"))
        .await
        .unwrap();

    assert_eq!(first.outcome, PublishOutcome::Stored);
    assert_eq!(again.outcome, PublishOutcome::Duplicate);
    assert_eq!(again.sequence, first.sequence);
    bus.delete_stream(&spec.name).await.unwrap();
}

#[tokio::test]
async fn live_subscribers_see_recorded_subjects_over_core_nats() {
    let bus = Bus::connect(NATS_URL).await.unwrap();
    let spec = spec("live");
    bus.ensure_stream(&spec).await.unwrap();
    let subject = spec.subjects[0].replace('>', "trade");
    let mut live = bus.client().subscribe(subject.clone()).await.unwrap();

    bus.publish(&subject, None, &batch("x:y:z")).await.unwrap();

    let message = live.next().await.unwrap();
    let decoded: TradeBatch = decode(message.headers.as_ref(), &message.payload).unwrap();
    assert_eq!(decoded.market, "x:y:z");
    bus.delete_stream(&spec.name).await.unwrap();
}

#[test]
fn decoding_checks_the_type_header() {
    let mut headers = async_nats::HeaderMap::new();
    headers.insert(
        orderflow_contracts::subjects::TYPE_HEADER,
        "orderflow.v1.TradeBatch",
    );

    let wrong: Result<GapClosed, _> = decode(Some(&headers), &[]);
    assert!(matches!(wrong, Err(DecodeError::UnexpectedType { .. })));

    let missing: Result<TradeBatch, _> = decode(None, &[]);
    assert!(matches!(missing, Err(DecodeError::MissingType)));
}

#[tokio::test]
async fn ensuring_a_stream_twice_is_idempotent() {
    let bus = Bus::connect(NATS_URL).await.unwrap();
    let spec = spec("idem");

    bus.ensure_stream(&spec).await.unwrap();
    bus.ensure_stream(&spec).await.unwrap();

    bus.delete_stream(&spec.name).await.unwrap();
}
