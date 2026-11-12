use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use orderflow_domain::{Bubble, Decimal, LargeBubbleRule, Market, TakerSide, TimeRange, Timestamp};
use orderflow_store::{BubbleQuery, StoreError};
use serde::Deserialize;

use crate::dto::decimal;
use crate::params::{ApiError, MarketParam, RangeParams};
use crate::{AppState, BubbleDto, BubblesResponse, GapDto, LargeBubbleDto, now};

const LARGE_WINDOW_US: i64 = 24 * 60 * 60 * 1_000_000;
const MINUTE_US: i64 = 60_000_000;
const MAX_SPAN_US: i64 = 3 * LARGE_WINDOW_US;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct BubbleParams {
    percentile: Option<String>,
    usd_floor: Option<String>,
    min_notional: Option<String>,
    side: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BubbleSpec {
    pub(crate) rule: LargeBubbleRule,
    pub(crate) query: BubbleQuery,
}

impl BubbleSpec {
    pub(crate) fn accepts(&self, bubble: &Bubble) -> bool {
        bubble.notional >= self.query.min_notional
            && self.query.side.is_none_or(|side| bubble.side == side)
    }
}

impl BubbleParams {
    pub(crate) fn validate(&self) -> Result<BubbleSpec, ApiError> {
        let defaults = LargeBubbleRule::default();
        let percentile = optional_decimal("percentile", self.percentile.as_deref())?
            .unwrap_or(defaults.percentile());
        let usd_floor = optional_decimal("usd_floor", self.usd_floor.as_deref())?;
        let rule = LargeBubbleRule::new(percentile, usd_floor)
            .map_err(|e| ApiError::bad_request(e.to_string()))?;
        let min_notional = optional_decimal("min_notional", self.min_notional.as_deref())?
            .unwrap_or(Decimal::ZERO);
        if min_notional.is_sign_negative() {
            return Err(ApiError::bad_request("min_notional must not be negative"));
        }
        let side = match self.side.as_deref() {
            None | Some("" | "both") => None,
            Some("buy") => Some(TakerSide::Buy),
            Some("sell") => Some(TakerSide::Sell),
            Some(other) => {
                return Err(ApiError::bad_request(format!(
                    "side `{other}` must be buy, sell or both"
                )));
            }
        };
        Ok(BubbleSpec {
            rule,
            query: BubbleQuery {
                min_notional,
                side,
                ..BubbleQuery::default()
            },
        })
    }
}

fn optional_decimal(name: &str, raw: Option<&str>) -> Result<Option<Decimal>, ApiError> {
    raw.filter(|r| !r.is_empty())
        .map(|r| {
            r.parse::<Decimal>()
                .map_err(|_| ApiError::bad_request(format!("{name} `{r}` is not a number")))
        })
        .transpose()
}

pub(crate) fn large_window(end: Timestamp) -> Option<TimeRange> {
    let end = end.as_micros().div_euclid(MINUTE_US) * MINUTE_US;
    TimeRange::new(
        Timestamp::from_micros(end.checked_sub(LARGE_WINDOW_US)?),
        Timestamp::from_micros(end),
    )
}

impl AppState {
    pub(crate) async fn large_threshold(
        &self,
        market: &Market,
        window: TimeRange,
        rule: &LargeBubbleRule,
    ) -> Result<Option<Decimal>, StoreError> {
        let count = self.store.bubble_count(market, window).await?;
        let key = (market.clone(), rule.percentile(), window.end(), count);
        if let Some(cached) = self.thresholds.lock().await.get(&key) {
            return Ok(*cached);
        }
        let threshold = self
            .store
            .large_bubble_threshold_of(market, window, rule, count)
            .await?;
        let mut cache = self.thresholds.lock().await;
        let stale = Timestamp::from_micros(window.end().as_micros() - 2 * MINUTE_US);
        cache.retain(|(_, _, end, _), _| *end > stale);
        cache.insert(key, threshold);
        Ok(threshold)
    }
}

pub(crate) fn large_dto(
    spec: &BubbleSpec,
    window: TimeRange,
    threshold: Option<Decimal>,
) -> LargeBubbleDto {
    LargeBubbleDto {
        percentile: decimal(spec.rule.percentile()),
        usd_floor: spec.rule.usd_floor().map(decimal),
        threshold: threshold.map(decimal),
        window_start_us: window.start().as_micros(),
        window_end_us: window.end().as_micros(),
    }
}

pub(crate) async fn bubbles(
    State(state): State<Arc<AppState>>,
    Query(market): Query<MarketParam>,
    Query(range): Query<RangeParams>,
    Query(params): Query<BubbleParams>,
) -> Result<Json<BubblesResponse>, ApiError> {
    let spec = state.market(&market.market)?;
    let range = range.validate(None)?;
    if range.end().as_micros() - range.start().as_micros() > MAX_SPAN_US {
        return Err(ApiError::bad_request("a Bubble range spans at most 3 days"));
    }
    let bubble = params.validate()?;
    let window = large_window(range.end().min(now()))
        .ok_or_else(|| ApiError::bad_request("to_us is out of range"))?;
    let (page, gaps, threshold) = tokio::try_join!(
        state.store.bubbles(&spec.market, range, &bubble.query),
        state.store.gaps(&spec.market, range),
        state.large_threshold(&spec.market, window, &bubble.rule),
    )?;
    Ok(Json(BubblesResponse {
        market: spec.market.to_string(),
        burst_grouped: !spec.market.venue().attributes_taker_orders(),
        large: large_dto(&bubble, window, threshold),
        bubbles: page
            .bubbles
            .iter()
            .map(|b| BubbleDto::new(b, bubble.rule.is_large(b.notional, threshold)))
            .collect(),
        truncated: page.truncated,
        gaps: gaps.iter().map(GapDto::from).collect(),
    }))
}
