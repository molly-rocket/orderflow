use orderflow_domain::{GapReason, Market, Timestamp, Trade};
use prost::{Message, Name};

use crate::convert::{self, ConvertError};
use crate::v1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    Trades {
        market: Market,
        trades: Vec<Trade>,
    },
    GapOpened {
        gap_id: String,
        market: Market,
        start: Timestamp,
        reason: GapReason,
    },
    GapClosed {
        gap_id: String,
        market: Market,
        end: Timestamp,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("unknown record type `{0}`")]
    UnknownType(String),
    #[error("payload does not decode: {0}")]
    Payload(#[from] prost::DecodeError),
    #[error(transparent)]
    Convert(#[from] ConvertError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoded {
    pub type_name: String,
    pub payload: Vec<u8>,
}

impl Record {
    pub fn market(&self) -> &Market {
        match self {
            Self::Trades { market, .. }
            | Self::GapOpened { market, .. }
            | Self::GapClosed { market, .. } => market,
        }
    }

    pub fn dedupe_id(&self) -> Option<String> {
        match self {
            Self::Trades { market, trades } => trades
                .first()
                .map(|t| format!("{market}/t/{}", t.venue_trade_id)),
            Self::GapOpened { gap_id, .. } => Some(format!("{gap_id}/open")),
            Self::GapClosed { gap_id, .. } => Some(format!("{gap_id}/close")),
        }
    }

    pub fn encode(&self) -> Result<Encoded, ConvertError> {
        Ok(match self {
            Self::Trades { market, trades } => {
                encoded(&convert::trade_batch_to_proto(market, trades)?)
            }
            Self::GapOpened {
                gap_id,
                market,
                start,
                reason,
            } => encoded(&v1::GapOpened {
                gap_id: gap_id.clone(),
                market: market.to_string(),
                start_time_us: start.as_micros(),
                reason: convert::gap_reason_to_proto(*reason).into(),
            }),
            Self::GapClosed {
                gap_id,
                market,
                end,
            } => encoded(&v1::GapClosed {
                gap_id: gap_id.clone(),
                market: market.to_string(),
                end_time_us: end.as_micros(),
            }),
        })
    }

    pub fn decode(type_name: &str, payload: &[u8]) -> Result<Self, RecordError> {
        if type_name == v1::TradeBatch::full_name() {
            let (market, trades) =
                convert::trade_batch_from_proto(v1::TradeBatch::decode(payload)?)?;
            Ok(Self::Trades { market, trades })
        } else if type_name == v1::GapOpened::full_name() {
            let g = v1::GapOpened::decode(payload)?;
            Ok(Self::GapOpened {
                reason: convert::gap_reason_from_proto(g.reason())?,
                market: g.market.parse().map_err(ConvertError::from)?,
                gap_id: g.gap_id,
                start: Timestamp::from_micros(g.start_time_us),
            })
        } else if type_name == v1::GapClosed::full_name() {
            let g = v1::GapClosed::decode(payload)?;
            Ok(Self::GapClosed {
                market: g.market.parse().map_err(ConvertError::from)?,
                gap_id: g.gap_id,
                end: Timestamp::from_micros(g.end_time_us),
            })
        } else {
            Err(RecordError::UnknownType(type_name.to_owned()))
        }
    }
}

fn encoded<M: Message + Name>(message: &M) -> Encoded {
    Encoded {
        type_name: M::full_name(),
        payload: message.encode_to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use orderflow_domain::{GapReason, TakerSide, Timestamp, Trade};

    use super::*;

    fn btc() -> Market {
        "binance:perp:BTCUSDT".parse().unwrap()
    }

    #[test]
    fn every_record_round_trips_through_its_wire_type() {
        let records = [
            Record::Trades {
                market: btc(),
                trades: vec![Trade {
                    venue_trade_id: 7,
                    engine_time: Timestamp::from_millis(1),
                    price: "75191.30".parse().unwrap(),
                    size: "0.086".parse().unwrap(),
                    taker_side: TakerSide::Buy,
                    taker_order_id: None,
                }],
            },
            Record::GapOpened {
                gap_id: "g".into(),
                market: btc(),
                start: Timestamp::from_micros(5),
                reason: GapReason::SequenceBreak,
            },
            Record::GapClosed {
                gap_id: "g".into(),
                market: btc(),
                end: Timestamp::from_micros(9),
            },
        ];
        for record in records {
            let wire = record.encode().unwrap();
            assert_eq!(
                Record::decode(&wire.type_name, &wire.payload).unwrap(),
                record
            );
        }
        assert!(matches!(
            Record::decode("orderflow.v1.Nope", &[]),
            Err(RecordError::UnknownType(_))
        ));
    }

    #[test]
    fn dedupe_ids_are_deterministic() {
        let opened = Record::GapOpened {
            gap_id: "binance:perp:BTCUSDT/42/Disconnect".into(),
            market: btc(),
            start: Timestamp::from_micros(5),
            reason: GapReason::Disconnect,
        };
        assert_eq!(
            opened.dedupe_id().as_deref(),
            Some("binance:perp:BTCUSDT/42/Disconnect/open")
        );
        let empty = Record::Trades {
            market: btc(),
            trades: vec![],
        };
        assert_eq!(empty.dedupe_id(), None);
    }
}
