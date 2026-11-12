use std::fmt;
use std::str::FromStr;

use rust_decimal::Decimal;

use crate::{PriceGroupingError, TickSize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Venue {
    Binance,
    Hyperliquid,
}

impl Venue {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Binance => "binance",
            Self::Hyperliquid => "hyperliquid",
        }
    }

    pub const fn attributes_taker_orders(self) -> bool {
        match self {
            Self::Binance => false,
            Self::Hyperliquid => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MarketType {
    Spot,
    Perp,
}

impl MarketType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Spot => "spot",
            Self::Perp => "perp",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Market {
    venue: Venue,
    market_type: MarketType,
    native_name: Box<str>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MarketIdError {
    #[error("market id must look like `venue:type:native-name`")]
    Malformed,
    #[error("unknown venue `{0}`")]
    UnknownVenue(String),
    #[error("unknown market type `{0}`")]
    UnknownMarketType(String),
    #[error("native name must not contain `.`, `*`, `>` or whitespace")]
    InvalidNativeName,
}

impl Market {
    pub fn new(
        venue: Venue,
        market_type: MarketType,
        native_name: &str,
    ) -> Result<Self, MarketIdError> {
        if native_name.is_empty() {
            return Err(MarketIdError::Malformed);
        }
        if native_name
            .chars()
            .any(|c| matches!(c, '.' | '*' | '>') || c.is_whitespace())
        {
            return Err(MarketIdError::InvalidNativeName);
        }
        Ok(Self {
            venue,
            market_type,
            native_name: native_name.into(),
        })
    }

    pub fn venue(&self) -> Venue {
        self.venue
    }

    pub fn market_type(&self) -> MarketType {
        self.market_type
    }

    pub fn native_name(&self) -> &str {
        &self.native_name
    }
}

impl FromStr for Venue {
    type Err = MarketIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "binance" => Ok(Self::Binance),
            "hyperliquid" => Ok(Self::Hyperliquid),
            other => Err(MarketIdError::UnknownVenue(other.to_owned())),
        }
    }
}

impl FromStr for MarketType {
    type Err = MarketIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "spot" => Ok(Self::Spot),
            "perp" => Ok(Self::Perp),
            other => Err(MarketIdError::UnknownMarketType(other.to_owned())),
        }
    }
}

impl FromStr for Market {
    type Err = MarketIdError;

    fn from_str(id: &str) -> Result<Self, Self::Err> {
        let mut parts = id.splitn(3, ':');
        let (Some(venue), Some(market_type), Some(native_name)) =
            (parts.next(), parts.next(), parts.next())
        else {
            return Err(MarketIdError::Malformed);
        };
        Self::new(venue.parse()?, market_type.parse()?, native_name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketSpec {
    pub market: Market,
    pub tick: TickSize,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MarketSpecError {
    #[error("market spec must look like `market=tick`")]
    Malformed,
    #[error(transparent)]
    Market(#[from] MarketIdError),
    #[error("tick `{0}` is not a decimal")]
    Tick(String),
    #[error(transparent)]
    Grouping(#[from] PriceGroupingError),
}

impl FromStr for MarketSpec {
    type Err = MarketSpecError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (market, tick) = s.rsplit_once('=').ok_or(MarketSpecError::Malformed)?;
        let tick: Decimal = tick
            .parse()
            .map_err(|_| MarketSpecError::Tick(tick.to_owned()))?;
        Ok(Self {
            market: market.parse()?,
            tick: TickSize::new(tick)?,
        })
    }
}

impl fmt::Display for Market {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            self.venue.as_str(),
            self.market_type.as_str(),
            self.native_name
        )
    }
}
