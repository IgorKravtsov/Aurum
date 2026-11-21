//! Python-compatible JSON scalars that serde would otherwise print differently.
//!
//! * [`PyFloat`] — a `float` response field. FastAPI renders floats with
//!   Python's `repr()` (`1e-05`, `1e+16`, `100.0`), while serde_json/ryu
//!   prints `0.00001`, `1e16`, `100.0`. Wrap every `float` field of a
//!   response DTO in `PyFloat`.
//! * [`PyDateTime`] — a timezone-aware `datetime` field. pydantic prints
//!   UTC as `Z` and omits the fraction when microseconds are zero
//!   (`2026-01-02T03:04:05Z`, `2026-01-02T03:04:05.000120Z`).
//! * Dates (`chrono::NaiveDate`) already serialise as `YYYY-MM-DD` and need
//!   no wrapper.

use std::fmt;

use chrono::{DateTime, SecondsFormat, Timelike, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Python's `float.__repr__` for a finite `f64`.
///
/// Both languages emit the shortest digit string that round-trips; only the
/// layout differs. Python uses fixed notation when `-4 <= exp10 < 16`
/// (always with a fractional part, `.0` if integral) and scientific
/// otherwise (`d.ddde±XX`, at least two exponent digits).
pub fn py_float_repr(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if value == 0.0 {
        return if value.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    // `{:e}` gives the shortest round-trip digits as "d.ddde<exp>".
    let sci = format!("{value:e}");
    let (mantissa, exp) = sci.split_once('e').expect("LowerExp always has an exponent");
    let exp: i32 = exp.parse().expect("exponent is an integer");
    let (negative, mantissa) = match mantissa.strip_prefix('-') {
        Some(m) => (true, m),
        None => (false, mantissa),
    };
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let sign = if negative { "-" } else { "" };
    if (-4..16).contains(&exp) {
        let point = exp + 1; // position of the decimal point within `digits`
        let body = if point <= 0 {
            format!("0.{}{}", "0".repeat((-point) as usize), digits)
        } else if point as usize >= digits.len() {
            format!("{}{}.0", digits, "0".repeat(point as usize - digits.len()))
        } else {
            let (head, tail) = digits.split_at(point as usize);
            format!("{head}.{tail}")
        };
        format!("{sign}{body}")
    } else {
        let (head, tail) = digits.split_at(1);
        let mantissa = if tail.is_empty() { head.to_string() } else { format!("{head}.{tail}") };
        let exp_sign = if exp < 0 { '-' } else { '+' };
        format!("{sign}{mantissa}e{exp_sign}{:02}", exp.abs())
    }
}

/// A `float` response field, serialised like Python's `json.dumps(float)`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct PyFloat(pub f64);

impl From<f64> for PyFloat {
    fn from(value: f64) -> Self {
        Self(value)
    }
}

impl fmt::Display for PyFloat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&py_float_repr(self.0))
    }
}

impl Serialize for PyFloat {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if !self.0.is_finite() {
            // json.dumps(allow_nan=False) raises → the Python app answers 500.
            return Err(serde::ser::Error::custom("non-finite float in response"));
        }
        let raw = serde_json::value::RawValue::from_string(py_float_repr(self.0)).map_err(serde::ser::Error::custom)?;
        raw.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PyFloat {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        f64::deserialize(deserializer).map(Self)
    }
}

/// A timezone-aware timestamp, serialised the way pydantic v2 prints an
/// asyncpg `timestamptz` (always UTC): `Z` suffix, microseconds only when
/// non-zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PyDateTime(pub DateTime<Utc>);

impl From<DateTime<Utc>> for PyDateTime {
    fn from(value: DateTime<Utc>) -> Self {
        Self(value)
    }
}

impl fmt::Display for PyDateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Postgres keeps microseconds, so anything finer is truncated first.
        let micros = self.0.nanosecond() / 1_000;
        let format = if micros == 0 { SecondsFormat::Secs } else { SecondsFormat::Micros };
        let truncated = self.0.with_nanosecond(micros * 1_000).unwrap_or(self.0);
        f.write_str(&truncated.to_rfc3339_opts(format, true))
    }
}

impl Serialize for PyDateTime {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for PyDateTime {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        DateTime::<Utc>::deserialize(deserializer).map(Self)
    }
}

#[cfg(feature = "sqlx")]
mod sqlx_impl {
    use super::PyDateTime;
    use chrono::{DateTime, Utc};
    use sqlx::encode::IsNull;
    use sqlx::error::BoxDynError;
    use sqlx::postgres::{PgArgumentBuffer, PgTypeInfo, PgValueRef, Postgres};

    impl sqlx::Type<Postgres> for PyDateTime {
        fn type_info() -> PgTypeInfo {
            <DateTime<Utc> as sqlx::Type<Postgres>>::type_info()
        }
    }

    impl<'r> sqlx::Decode<'r, Postgres> for PyDateTime {
        fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
            Ok(PyDateTime(<DateTime<Utc> as sqlx::Decode<Postgres>>::decode(value)?))
        }
    }

    impl sqlx::Encode<'_, Postgres> for PyDateTime {
        fn encode_by_ref(&self, buf: &mut PgArgumentBuffer) -> Result<IsNull, BoxDynError> {
            <DateTime<Utc> as sqlx::Encode<Postgres>>::encode_by_ref(&self.0, buf)
        }
    }
}

#[cfg(feature = "openapi")]
mod openapi_impl {
    use super::{PyDateTime, PyFloat};
    use utoipa::openapi::RefOr;
    use utoipa::openapi::schema::{KnownFormat, ObjectBuilder, Schema, SchemaFormat, Type};

    impl utoipa::PartialSchema for PyFloat {
        fn schema() -> RefOr<Schema> {
            ObjectBuilder::new().schema_type(Type::Number).into()
        }
    }
    impl utoipa::ToSchema for PyFloat {}

    impl utoipa::PartialSchema for PyDateTime {
        fn schema() -> RefOr<Schema> {
            ObjectBuilder::new()
                .schema_type(Type::String)
                .format(Some(SchemaFormat::KnownFormat(KnownFormat::DateTime)))
                .into()
        }
    }
    impl utoipa::ToSchema for PyDateTime {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn float_repr_matches_python() {
        for (value, expected) in [
            (1e-5, "1e-05"),
            (1e16, "1e+16"),
            (100.0, "100.0"),
            (0.1 + 0.2, "0.30000000000000004"),
            (12.5, "12.5"),
            (-0.0, "-0.0"),
            (0.0001, "0.0001"),
            (1.5e-7, "1.5e-07"),
            (123456789012345.6, "123456789012345.6"),
            (1234567890123456.7, "1234567890123456.8"),
            (12345678901234567.0, "1.2345678901234568e+16"),
            (33.333333333333336, "33.333333333333336"),
            (-42.0, "-42.0"),
        ] {
            assert_eq!(py_float_repr(value), expected, "{value}");
        }
    }

    #[test]
    fn datetime_matches_pydantic() {
        let t = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
        assert_eq!(PyDateTime(t).to_string(), "2026-01-02T03:04:05Z");
        let t = t.with_nanosecond(120_000).unwrap();
        assert_eq!(PyDateTime(t).to_string(), "2026-01-02T03:04:05.000120Z");
    }

    #[test]
    fn float_serialises_raw() {
        let json = serde_json::to_string(&vec![PyFloat(1e-5), PyFloat(2.0)]).unwrap();
        assert_eq!(json, "[1e-05,2.0]");
    }
}
