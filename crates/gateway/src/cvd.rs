use orderflow_domain::{Cvd, Decimal, Gap, Market, TimeRange, Timestamp};
use orderflow_store::{Store, StoreError};

pub(crate) fn since_trading_day(first_bar: Timestamp, end: Timestamp) -> Option<TimeRange> {
    TimeRange::new(first_bar.trading_day_start(), end)
}

pub(crate) async fn opening(
    store: &Store,
    market: &Market,
    first_bar: Timestamp,
    gaps_since_day: &[Gap],
) -> Result<Cvd, StoreError> {
    let Some(before) = TimeRange::new(first_bar.trading_day_start(), first_bar) else {
        return Ok(Cvd::opening(first_bar, Decimal::ZERO, true));
    };
    let delta = store.taker_delta(market, before).await?;
    Ok(Cvd::opening(
        first_bar,
        delta,
        !Gap::any_overlaps(gaps_since_day, before),
    ))
}
