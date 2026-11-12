use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};

use crate::cvd;
use crate::params::{ApiError, BarParams, MarketParam, RangeParams};
use crate::{AppState, BarDto, FootprintResponse, GapDto, MarketDto};

pub(crate) async fn markets(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<MarketDto>>, ApiError> {
    let mut out = Vec::with_capacity(state.markets.len());
    for spec in state.markets.values() {
        let market = &spec.market;
        let horizon = state.store.recording_horizon(market).await?;
        out.push(MarketDto {
            id: market.to_string(),
            venue: market.venue().as_str(),
            market_type: market.market_type().as_str(),
            native_name: market.native_name().to_owned(),
            tick_size: spec.tick.get().normalize().to_string(),
            recording_horizon_us: horizon.map(orderflow_domain::Timestamp::as_micros),
            burst_grouped: !market.venue().attributes_taker_orders(),
        });
    }
    Ok(Json(out))
}

pub(crate) async fn footprint(
    State(state): State<Arc<AppState>>,
    Query(market): Query<MarketParam>,
    Query(range): Query<RangeParams>,
    Query(bar): Query<BarParams>,
) -> Result<Json<FootprintResponse>, ApiError> {
    let spec = state.market(&market.market)?;
    let bar = bar.validate(spec)?;
    let range = range.validate(Some(bar.timeframe))?;
    let first_bar = bar.timeframe.bar_start(range.start());
    let since_day = cvd::since_trading_day(first_bar, range.end()).unwrap_or(range);

    let (bars, day_gaps) = tokio::try_join!(
        state
            .store
            .footprint_bars(&spec.market, range, bar.timeframe, bar.grouping),
        state.store.gaps(&spec.market, since_day),
    )?;
    let mut cvd = cvd::opening(&state.store, &spec.market, first_bar, &day_gaps).await?;
    Ok(Json(FootprintResponse {
        market: spec.market.to_string(),
        timeframe_secs: bar.timeframe.as_secs(),
        grouping: bar.grouping.into(),
        imbalance: bar.imbalance.into(),
        bars: bars
            .iter()
            .map(|b| {
                let point = cvd.advance(b.range(), b.delta(), &day_gaps);
                BarDto::new(b, &bar.imbalance, &day_gaps, point)
            })
            .collect(),
        gaps: day_gaps
            .iter()
            .filter(|g| g.overlaps(range))
            .map(GapDto::from)
            .collect(),
    }))
}

pub(crate) async fn gaps(
    State(state): State<Arc<AppState>>,
    Query(market): Query<MarketParam>,
    Query(range): Query<RangeParams>,
) -> Result<Json<Vec<GapDto>>, ApiError> {
    let spec = state.market(&market.market)?;
    let range = range.validate(None)?;
    let gaps = state.store.gaps(&spec.market, range).await?;
    Ok(Json(gaps.iter().map(GapDto::from).collect()))
}
