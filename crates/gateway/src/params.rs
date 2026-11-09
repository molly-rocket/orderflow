use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use orderflow_domain::{
    Decimal, ImbalanceSettings, Market, PriceGrouping, TimeRange, Timeframe, Timestamp,
};
use serde::Deserialize;

use crate::{AppState, ErrorDto, MarketSpec};

pub(crate) const MAX_BARS: i64 = 5_000;

const RETRY_AFTER_SECS: u32 = 1;

#[derive(Debug)]
pub(crate) struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    pub(crate) fn storage_unavailable(error: impl std::fmt::Display) -> Self {
        tracing::warn!(%error, "storage unavailable");
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "storage unavailable".into(),
        }
    }

    pub(crate) fn internal(error: impl std::fmt::Display) -> Self {
        tracing::error!(%error, "request failed");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "internal error".into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (
            self.status,
            Json(ErrorDto {
                error: self.message,
            }),
        )
            .into_response();
        if self.status == StatusCode::SERVICE_UNAVAILABLE {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(RETRY_AFTER_SECS));
        }
        response
    }
}

impl From<orderflow_store::StoreError> for ApiError {
    fn from(error: orderflow_store::StoreError) -> Self {
        match error {
            orderflow_store::StoreError::UnsupportedTimeframe => {
                Self::bad_request(error.to_string())
            }
            unavailable if unavailable.is_unavailable() => Self::storage_unavailable(unavailable),
            other => Self::internal(other),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct MarketParam {
    pub(crate) market: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RangeParams {
    pub(crate) from_us: i64,
    pub(crate) to_us: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BarParams {
    #[serde(default = "default_timeframe")]
    pub(crate) timeframe_secs: u32,
    #[serde(default = "default_grouping")]
    pub(crate) grouping: u32,
    pub(crate) imbalance_ratio: Option<String>,
    pub(crate) min_stack: Option<u8>,
}

const fn default_timeframe() -> u32 {
    60
}

const fn default_grouping() -> u32 {
    1
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BarSpec {
    pub(crate) timeframe: Timeframe,
    pub(crate) grouping: PriceGrouping,
    pub(crate) imbalance: ImbalanceSettings,
}

impl AppState {
    pub(crate) fn market(&self, id: &str) -> Result<&MarketSpec, ApiError> {
        let market: Market = id
            .parse()
            .map_err(|e| ApiError::bad_request(format!("market `{id}`: {e}")))?;
        self.markets
            .get(&market)
            .ok_or_else(|| ApiError::not_found(format!("market `{id}` is not served")))
    }
}

impl BarParams {
    pub(crate) fn validate(&self, spec: &MarketSpec) -> Result<BarSpec, ApiError> {
        let timeframe = Timeframe::from_secs(self.timeframe_secs)
            .ok()
            .filter(|t| t.as_secs() % 60 == 0)
            .ok_or_else(|| {
                ApiError::bad_request(
                    "timeframe_secs must be a whole number of minutes that divides 24h",
                )
            })?;
        let grouping = PriceGrouping::new(spec.tick, self.grouping)
            .map_err(|e| ApiError::bad_request(format!("grouping: {e}")))?;
        let defaults = ImbalanceSettings::default();
        let ratio = match &self.imbalance_ratio {
            Some(raw) => raw.parse::<Decimal>().map_err(|_| {
                ApiError::bad_request(format!("imbalance_ratio `{raw}` is not a number"))
            })?,
            None => defaults.ratio(),
        };
        let imbalance =
            ImbalanceSettings::new(ratio, self.min_stack.unwrap_or(defaults.min_stack()))
                .map_err(|e| ApiError::bad_request(e.to_string()))?;
        Ok(BarSpec {
            timeframe,
            grouping,
            imbalance,
        })
    }
}

impl RangeParams {
    pub(crate) fn validate(&self, timeframe: Option<Timeframe>) -> Result<TimeRange, ApiError> {
        let range = TimeRange::new(
            Timestamp::from_micros(self.from_us),
            Timestamp::from_micros(self.to_us),
        )
        .ok_or_else(|| ApiError::bad_request("to_us must be after from_us"))?;
        if let Some(timeframe) = timeframe {
            let span = range.end().as_micros() - timeframe.bar_start(range.start()).as_micros();
            let bars = span / (i64::from(timeframe.as_secs()) * 1_000_000);
            if bars > MAX_BARS {
                return Err(ApiError::bad_request(format!(
                    "range spans {bars} bars; at most {MAX_BARS} per request"
                )));
            }
        }
        Ok(range)
    }
}
