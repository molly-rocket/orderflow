use std::time::{Duration, Instant};

use futures_util::StreamExt;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use venue_sim::{FakeVenue, Fault, Fixture, Frame, Pacing, Script};

fn fixture(n: usize) -> Fixture {
    Fixture::new(
        (0..n)
            .map(|i| Frame {
                recv_us: i64::try_from(i).unwrap() * 10_000,
                text: format!("f{i}"),
            })
            .collect(),
    )
}

async fn read_session(url: &str, limit: usize) -> Vec<String> {
    let (mut ws, _) = connect_async(url).await.expect("connect to fake venue");
    let mut got = Vec::new();
    while got.len() < limit {
        let Ok(next) = tokio::time::timeout(Duration::from_secs(5), ws.next()).await else {
            panic!("timed out after {got:?}");
        };
        match next {
            Some(Ok(Message::Text(text))) => got.push(text.to_string()),
            Some(Ok(Message::Close(_)) | Err(_)) | None => break,
            Some(Ok(_)) => {}
        }
    }
    got
}

fn names(range: std::ops::Range<usize>) -> Vec<String> {
    range.map(|i| format!("f{i}")).collect()
}

#[tokio::test]
async fn replays_every_frame_in_order() {
    let venue = FakeVenue::start(Script::new(fixture(5))).await.unwrap();

    assert_eq!(read_session(&venue.url(), 5).await, names(0..5));
    venue.finished().await;
}

#[tokio::test]
async fn a_disconnect_closes_the_session_and_loses_frames_sent_while_down() {
    let script = Script::new(fixture(10)).with_fault(Fault::Disconnect {
        after_frames: 3,
        lost_frames: 2,
    });
    let venue = FakeVenue::start(script).await.unwrap();

    assert_eq!(read_session(&venue.url(), 100).await, names(0..3));
    assert_eq!(read_session(&venue.url(), 5).await, names(5..10));
    assert_eq!(venue.connections(), 2);
}

#[tokio::test]
async fn dropped_frames_are_never_sent() {
    let script = Script::new(fixture(6)).with_fault(Fault::Drop {
        at_frame: 2,
        count: 2,
    });
    let venue = FakeVenue::start(script).await.unwrap();

    assert_eq!(
        read_session(&venue.url(), 4).await,
        vec!["f0", "f1", "f4", "f5"]
    );
}

#[tokio::test]
async fn realtime_pacing_follows_the_recorded_spacing() {
    let script = Script::new(fixture(5)).with_pacing(Pacing::Realtime { speed: 2.0 });
    let venue = FakeVenue::start(script).await.unwrap();

    let started = Instant::now();
    read_session(&venue.url(), 5).await;
    let elapsed = started.elapsed();

    assert!(elapsed >= Duration::from_millis(20), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(1), "{elapsed:?}");
}

#[tokio::test]
async fn the_requested_path_is_recorded_for_assertions() {
    let venue = FakeVenue::start(Script::new(fixture(1))).await.unwrap();

    read_session(
        &format!("{}/stream?streams=btcusdt@aggTrade", venue.url()),
        1,
    )
    .await;

    assert_eq!(
        venue.requested_paths(),
        vec!["/stream?streams=btcusdt@aggTrade"]
    );
}

#[test]
fn fixtures_round_trip_through_json_lines() {
    let original = fixture(3);
    let mut buf = Vec::new();
    original.write_jsonl(&mut buf).unwrap();

    let text = String::from_utf8(buf).unwrap();
    assert_eq!(text.lines().count(), 3);
    assert_eq!(Fixture::parse_jsonl(&text).unwrap(), original);
}
