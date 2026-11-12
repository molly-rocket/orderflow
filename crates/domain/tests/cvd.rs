mod common;

use common::d;
use orderflow_domain::{Cvd, CvdPoint, Decimal, Gap, GapReason, TimeRange, Timestamp};

const MINUTE_US: i64 = 60_000_000;
const DAY_US: i64 = 86_400_000_000;
const DAY_2: i64 = 20_000 * DAY_US;

fn minute(start_us: i64) -> TimeRange {
    TimeRange::new(
        Timestamp::from_micros(start_us),
        Timestamp::from_micros(start_us + MINUTE_US),
    )
    .unwrap()
}

fn point(value: &str, complete: bool) -> CvdPoint {
    CvdPoint {
        value: d(value),
        complete,
    }
}

fn run(cvd: &mut Cvd, bars: &[(i64, &str)], gaps: &[Gap]) -> Vec<CvdPoint> {
    bars.iter()
        .map(|(start, delta)| cvd.advance(minute(*start), d(delta), gaps))
        .collect()
}

#[test]
fn cvd_accumulates_taker_delta_through_the_trading_day() {
    let mut cvd = Cvd::opening(Timestamp::from_micros(DAY_2), Decimal::ZERO, true);

    let points = run(
        &mut cvd,
        &[
            (DAY_2, "1.5"),
            (DAY_2 + MINUTE_US, "-4"),
            (DAY_2 + 2 * MINUTE_US, "0.25"),
        ],
        &[],
    );

    assert_eq!(
        points,
        vec![
            point("1.5", true),
            point("-2.5", true),
            point("-2.25", true)
        ]
    );
}

#[test]
fn cvd_resets_at_00_00_utc() {
    let mut cvd = Cvd::opening(Timestamp::from_micros(DAY_2 - 2 * MINUTE_US), d("10"), true);

    let points = run(
        &mut cvd,
        &[
            (DAY_2 - 2 * MINUTE_US, "1"),
            (DAY_2 - MINUTE_US, "2"),
            (DAY_2, "-3"),
            (DAY_2 + MINUTE_US, "1"),
        ],
        &[],
    );

    assert_eq!(
        points,
        vec![
            point("11", true),
            point("13", true),
            point("-3", true),
            point("-2", true)
        ]
    );
}

#[test]
fn a_gap_makes_cvd_incomplete_until_the_next_trading_day() {
    let gap = Gap::closed(
        Timestamp::from_micros(DAY_2 - 3 * MINUTE_US + 10),
        Timestamp::from_micros(DAY_2 - 3 * MINUTE_US + 20),
        GapReason::Disconnect,
    )
    .unwrap();
    let mut cvd = Cvd::opening(Timestamp::from_micros(DAY_2 - 4 * MINUTE_US), d("0"), true);

    let points = run(
        &mut cvd,
        &[
            (DAY_2 - 4 * MINUTE_US, "1"),
            (DAY_2 - 3 * MINUTE_US, "1"),
            (DAY_2 - 2 * MINUTE_US, "1"),
            (DAY_2, "5"),
        ],
        &[gap],
    );

    assert_eq!(
        points,
        vec![
            point("1", true),
            point("2", false),
            point("3", false),
            point("5", true)
        ]
    );
}

#[test]
fn opening_mid_day_carries_the_value_and_completeness_so_far() {
    let mut cvd = Cvd::opening(
        Timestamp::from_micros(DAY_2 + 30 * MINUTE_US),
        d("-7"),
        false,
    );

    let points = run(&mut cvd, &[(DAY_2 + 30 * MINUTE_US, "2")], &[]);

    assert_eq!(points, vec![point("-5", false)]);
}

#[test]
fn the_trading_day_starts_at_00_00_utc() {
    let noon = Timestamp::from_micros(DAY_2 + DAY_US / 2);

    assert_eq!(noon.trading_day_start(), Timestamp::from_micros(DAY_2));
    assert_eq!(
        Timestamp::from_micros(DAY_2).trading_day_start(),
        Timestamp::from_micros(DAY_2)
    );
    assert_eq!(
        Timestamp::from_micros(DAY_2 - 1).trading_day_start(),
        Timestamp::from_micros(DAY_2 - DAY_US)
    );
}
