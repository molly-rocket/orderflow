use orderflow_domain::Decimal;

pub(crate) const SCALE: u32 = 18;

#[derive(Debug, thiserror::Error)]
#[error("{value} has more than {SCALE} decimal places; refusing to round")]
pub struct PrecisionError {
    value: Decimal,
}

pub(crate) fn to_clickhouse(value: Decimal) -> Result<i128, PrecisionError> {
    let scale = value.scale();
    if scale > SCALE {
        let normalized = value.normalize();
        if normalized.scale() > SCALE {
            return Err(PrecisionError { value });
        }
        return to_clickhouse(normalized);
    }
    Ok(value.mantissa() * 10_i128.pow(SCALE - scale))
}

pub(crate) fn from_clickhouse(raw: i128) -> Result<Decimal, crate::StoreError> {
    Decimal::try_from_i128_with_scale(raw, SCALE)
        .map(|d| d.normalize())
        .map_err(|_| crate::StoreError::Corrupt(format!("decimal {raw}e-18 out of range")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_exactly() {
        for s in [
            "0",
            "75191.30",
            "0.086",
            "-1.5",
            "123456789.123456789012345678",
        ] {
            let d: Decimal = s.parse().unwrap();
            assert_eq!(
                from_clickhouse(to_clickhouse(d).unwrap()).unwrap(),
                d,
                "{s}"
            );
        }
    }

    #[test]
    fn excess_precision_is_refused_unless_it_is_trailing_zeros() {
        assert!(to_clickhouse("0.0000000000000000001".parse().unwrap()).is_err());
        assert_eq!(
            to_clickhouse("1.0000000000000000000000".parse().unwrap()).unwrap(),
            10_i128.pow(18)
        );
    }
}
