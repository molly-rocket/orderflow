use orderflow_domain::{Gap, GapReason, TimeRange, Timestamp};

fn t(micros: i64) -> Timestamp {
    Timestamp::from_micros(micros)
}

fn range(start: i64, end: i64) -> TimeRange {
    TimeRange::new(t(start), t(end)).unwrap()
}

#[test]
fn a_closed_gap_overlaps_ranges_that_share_any_instant_with_it() {
    let gap = Gap::closed(t(100), t(200), GapReason::Disconnect).unwrap();
    let cases = [
        ((0, 100), false),
        ((0, 101), true),
        ((150, 160), true),
        ((50, 250), true),
        ((199, 300), true),
        ((200, 300), false),
    ];
    for ((start, end), expected) in cases {
        assert_eq!(
            gap.overlaps(range(start, end)),
            expected,
            "[{start}, {end})"
        );
    }
}

#[test]
fn an_open_gap_extends_to_the_future() {
    let gap = Gap::open(t(100), GapReason::SequenceBreak);

    assert!(gap.is_open());
    assert!(gap.overlaps(range(1_000_000, 2_000_000)));
    assert!(!gap.overlaps(range(0, 100)));
}

#[test]
fn closing_a_gap_must_not_end_before_it_starts() {
    assert!(Gap::closed(t(200), t(100), GapReason::Disconnect).is_none());
    assert!(Gap::closed(t(100), t(100), GapReason::Disconnect).is_some());
    assert!(
        Gap::open(t(100), GapReason::BookResync)
            .close(t(50))
            .is_none()
    );
    assert_eq!(
        Gap::open(t(100), GapReason::BookResync).close(t(150)),
        Gap::closed(t(100), t(150), GapReason::BookResync)
    );
}

#[test]
fn a_bar_is_incomplete_when_any_gap_overlaps_it() {
    let gaps = [
        Gap::closed(t(100), t(200), GapReason::Disconnect).unwrap(),
        Gap::closed(t(500), t(600), GapReason::RecorderDowntime).unwrap(),
    ];

    assert!(Gap::any_overlaps(&gaps, range(150, 160)));
    assert!(!Gap::any_overlaps(&gaps, range(200, 500)));
    assert!(Gap::any_overlaps(&gaps, range(590, 700)));
}
