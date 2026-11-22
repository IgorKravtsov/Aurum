//! pydantic's lax `datetime` validation (speedate + pydantic-core), the
//! subset a backup file can reach: RFC 3339-ish strings, date-only strings,
//! unix timestamps (numbers or numeric strings, seconds or milliseconds).
//!
//! Kept private to the backup module: it is the only request DTO with a
//! `datetime` field so far. Promote it into `validation` once a second
//! group needs it.

use chrono::{DateTime, Duration, FixedOffset, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use serde_json::{Value, json};

use crate::pyfmt::PyDateTime;
use crate::validation::{Checked, ErrorItem, Loc, Res, Val};

/// Seconds above which speedate reads a timestamp as milliseconds.
const MS_WATERSHED: i64 = 20_000_000_000;
/// `0000-01-01T00:00:00Z` and `9999-12-31T23:59:59Z` as unix seconds.
const UNIX_0000: i64 = -62_167_219_200;
const UNIX_9999: i64 = 253_402_300_799;
/// `0001-01-01T00:00:00Z` — Python's `datetime.min`.
const UNIX_0001: i64 = -62_135_596_800;

/// A parsed pydantic `datetime`: naive unless the input carried an offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParsedDateTime {
    pub naive: NaiveDateTime,
    pub offset: Option<FixedOffset>,
}

impl ParsedDateTime {
    /// The instant asyncpg would store for this value in a `timestamptz`
    /// column: an aware value is converted to UTC, a naive one is taken as
    /// the server's *local* time (`datetime.astimezone()` semantics).
    pub fn to_utc(self) -> DateTime<Utc> {
        match self.offset {
            Some(offset) => Utc.from_utc_datetime(&(self.naive - Duration::seconds(offset.local_minus_utc().into()))),
            None => Local
                .from_local_datetime(&self.naive)
                .earliest()
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|| Utc.from_utc_datetime(&self.naive)),
        }
    }
}

enum Failure {
    /// `datetime_parsing` — the input was understood but is out of range.
    Parsing(&'static str),
    /// `datetime_from_date_parsing` — the input is not a datetime, and not a
    /// date either; carries the date parser's complaint.
    FromDate(&'static str),
}

fn error(loc: Loc, input: Value, failure: Failure) -> ErrorItem {
    match failure {
        Failure::Parsing(e) => {
            ErrorItem::new("datetime_parsing", loc, format!("Input should be a valid datetime, {e}"), input)
                .with_ctx(json!({ "error": e }))
        }
        Failure::FromDate(e) => ErrorItem::new(
            "datetime_from_date_parsing",
            loc,
            format!("Input should be a valid datetime or date, {e}"),
            input,
        )
        .with_ctx(json!({ "error": e })),
    }
}

/// `datetime` field validator; the value is resolved to UTC like asyncpg
/// would when writing it (see [`ParsedDateTime::to_utc`]).
pub(crate) fn datetime<'a>(v: &Val<'a>) -> Res<'a, PyDateTime> {
    let parsed = match v.value {
        Value::Number(n) => {
            let parsed = if let Some(i) = n.as_i64() {
                from_timestamp(i, 0)
            } else {
                // pydantic-core's float_as_datetime: floor for the seconds,
                // |fract| for the micros (sic — so -1.25 lands on -1.75).
                let f = n.as_f64().unwrap_or(f64::NAN);
                let micros = (f.fract().abs() * 1_000_000.0).round() as u32;
                from_timestamp(f.floor() as i64, micros)
            };
            parsed.map_err(Failure::Parsing)
        }
        Value::String(s) => parse_str(s),
        _ => {
            return Err(ErrorItem::new(
                "datetime_type",
                v.loc.clone(),
                "Input should be a valid datetime",
                v.value.clone(),
            )
            .into());
        }
    };
    match parsed.and_then(check_year) {
        Ok(p) => Ok(Checked { value: PyDateTime(p.to_utc()), input: v.value, loc: v.loc.clone() }),
        Err(failure) => Err(error(v.loc.clone(), v.value.clone(), failure).into()),
    }
}

/// Python's `datetime` has no year 0, speedate does.
fn check_year(p: ParsedDateTime) -> Result<ParsedDateTime, Failure> {
    use chrono::Datelike;
    if p.naive.year() == 0 { Err(Failure::Parsing("year 0 is out of range")) } else { Ok(p) }
}

fn parse_str(s: &str) -> Result<ParsedDateTime, Failure> {
    if let Some(p) = parse_rfc3339(s.as_bytes()) {
        return Ok(p);
    }
    let timestamp = match parse_number(s.as_bytes()) {
        Some(Number::Int(i)) => from_timestamp(i, 0).ok(),
        Some(Number::Float(f)) => {
            let floor = f.floor();
            from_timestamp(floor as i64, ((f - floor) * 1_000_000.0).round() as u32).ok()
        }
        None => None,
    };
    if let Some(p) = timestamp {
        return Ok(p);
    }
    parse_date(s.as_bytes())
        .map(|date| ParsedDateTime { naive: date.and_time(NaiveTime::MIN), offset: None })
        .map_err(Failure::FromDate)
}

/// speedate's `DateTime::from_timestamp`: seconds, or milliseconds when the
/// magnitude is above [`MS_WATERSHED`].
fn from_timestamp(timestamp: i64, micros: u32) -> Result<ParsedDateTime, &'static str> {
    let abs = timestamp.checked_abs().ok_or("dates before 0000 are not supported as unix timestamps")?;
    let (seconds, extra_micros, micros) = if abs <= MS_WATERSHED {
        (timestamp, 0, micros)
    } else {
        (timestamp.div_euclid(1_000), timestamp.rem_euclid(1_000) as u32 * 1_000, micros / 1_000)
    };
    if seconds < UNIX_0000 {
        return Err("dates before 0000 are not supported as unix timestamps");
    }
    if seconds > UNIX_9999 {
        return Err("dates after 9999 are not supported as unix timestamps");
    }
    if seconds < UNIX_0001 {
        return Err("year 0 is out of range");
    }
    let total = Duration::seconds(seconds) + Duration::microseconds(i64::from(extra_micros + micros));
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("valid date").and_time(NaiveTime::MIN);
    Ok(ParsedDateTime { naive: epoch + total, offset: Some(FixedOffset::east_opt(0).expect("zero offset")) })
}

enum Number {
    Int(i64),
    Float(f64),
}

/// speedate's `float_parse_bytes`: `[+-]digits[.anything f64 accepts]`.
fn parse_number(b: &[u8]) -> Option<Number> {
    let s = std::str::from_utf8(b).ok()?;
    let unsigned = s.strip_prefix(['+', '-']).unwrap_or(s);
    let negative = s.starts_with('-');
    let int_len = unsigned.bytes().take_while(u8::is_ascii_digit).count();
    match unsigned.as_bytes().get(int_len) {
        None => {
            if int_len == 0 {
                return None;
            }
            let mut value: i64 = 0;
            for d in unsigned.bytes() {
                value = value.checked_mul(10)?.checked_add(i64::from(d - b'0'))?;
            }
            Some(Number::Int(if negative { -value } else { value }))
        }
        Some(b'.') => s.parse::<f64>().ok().map(Number::Float),
        Some(_) => None,
    }
}

fn digits(b: &[u8], start: usize, len: usize) -> Option<u32> {
    let slice = b.get(start..start + len)?;
    if !slice.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(slice.iter().fold(0, |acc, d| acc * 10 + u32::from(d - b'0')))
}

/// speedate's `Date::parse_bytes` with its error wording.
fn parse_date(b: &[u8]) -> Result<NaiveDate, &'static str> {
    if b.len() < 10 {
        return Err("input is too short");
    }
    let year = digits(b, 0, 4).ok_or("invalid character in year")?;
    if b[4] != b'-' {
        return Err("invalid date separator, expected `-`");
    }
    let month = digits(b, 5, 2).ok_or("invalid character in month")?;
    if b[7] != b'-' {
        return Err("invalid date separator, expected `-`");
    }
    let day = digits(b, 8, 2).ok_or("invalid character in day")?;
    if !(1..=12).contains(&month) {
        return Err("month value is outside expected range of 1-12");
    }
    let date = date_from_parts(year, month, day).ok_or("day value is outside expected range")?;
    if b.len() > 10 {
        return Err("unexpected extra characters at the end of the input");
    }
    Ok(date)
}

/// Year 0 is a valid proleptic Gregorian year for chrono (and speedate);
/// [`check_year`] rejects it afterwards with Python's message.
fn date_from_parts(year: u32, month: u32, day: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(i32::try_from(year).ok()?, month, day)
}

/// speedate's RFC 3339 parser: `YYYY-MM-DD[Tt _]HH:MM[:SS[(.|,)frac]][Z|z|±HH[:]MM]`.
/// `None` on any deviation — the caller then falls back to a timestamp or a
/// bare date, which produce the error message.
fn parse_rfc3339(b: &[u8]) -> Option<ParsedDateTime> {
    if b.len() < 16 {
        return None;
    }
    let date = parse_date(&b[..10]).ok()?;
    if !matches!(b[10], b'T' | b't' | b' ' | b'_') {
        return None;
    }
    let hour = digits(b, 11, 2)?;
    if b[13] != b':' {
        return None;
    }
    let minute = digits(b, 14, 2)?;
    let mut i = 16;
    let mut second = 0;
    let mut micros = 0;
    if b.get(i) == Some(&b':') {
        second = digits(b, i + 1, 2)?;
        i += 3;
        if matches!(b.get(i), Some(b'.' | b',')) {
            i += 1;
            let frac_len = b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
            if frac_len == 0 {
                return None;
            }
            // Digits past the sixth are dropped, not rounded.
            for k in 0..6 {
                micros = micros * 10 + b.get(i + k).filter(|_| k < frac_len).map_or(0, |d| u32::from(d - b'0'));
            }
            i += frac_len;
        }
    }
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let offset = match b.get(i) {
        None => None,
        Some(b'Z' | b'z') => {
            i += 1;
            Some(0)
        }
        Some(sign @ (b'+' | b'-')) => {
            let oh = digits(b, i + 1, 2)?;
            i += 3;
            if b.get(i) == Some(&b':') {
                i += 1;
            }
            let om = digits(b, i, 2)?;
            i += 2;
            if oh > 23 || om > 59 {
                return None;
            }
            let secs = i32::try_from(oh * 3_600 + om * 60).ok()?;
            Some(if *sign == b'-' { -secs } else { secs })
        }
        Some(_) => return None,
    };
    if i != b.len() {
        return None;
    }
    let time = NaiveTime::from_hms_micro_opt(hour, minute, second, micros)?;
    Some(ParsedDateTime {
        naive: date.and_time(time),
        offset: match offset {
            Some(secs) => Some(FixedOffset::east_opt(secs)?),
            None => None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::Source;

    fn check(input: Value) -> String {
        let v = Val { value: &input, loc: vec![], source: Source::Body };
        match datetime(&v) {
            Ok(c) => c.value.to_string(),
            Err(e) => format!("{}: {}", e.0[0].kind, e.0[0].msg),
        }
    }

    #[test]
    fn accepts_what_pydantic_accepts() {
        assert_eq!(check(json!("2026-01-01T10:00:00Z")), "2026-01-01T10:00:00Z");
        assert_eq!(check(json!("2026-01-01 10:00:00.123+02:00")), "2026-01-01T08:00:00.123000Z");
        assert_eq!(check(json!("2026-01-01T10:00:00.1234567Z")), "2026-01-01T10:00:00.123456Z");
        assert_eq!(check(json!("2026-01-01T10:00:00,5z")), "2026-01-01T10:00:00.500000Z");
        assert_eq!(check(json!("2026-01-01T10:00:00+0200")), "2026-01-01T08:00:00Z");
        assert_eq!(check(json!("2026-01-01T10:00Z")), "2026-01-01T10:00:00Z");
        assert_eq!(check(json!(1_700_000_000)), "2023-11-14T22:13:20Z");
        assert_eq!(check(json!("1700000000.5")), "2023-11-14T22:13:20.500000Z");
        assert_eq!(check(json!("-1.25")), "1969-12-31T23:59:58.750000Z");
        assert_eq!(check(json!(-1.25)), "1969-12-31T23:59:58.250000Z");
        assert_eq!(check(json!(20_000_000_001_i64)), "1970-08-20T11:33:20.001000Z");
        assert_eq!(check(json!(253_402_300_799_999_i64)), "9999-12-31T23:59:59.999000Z");
    }

    #[test]
    fn rejects_like_pydantic() {
        let from_date = "datetime_from_date_parsing: Input should be a valid datetime or date, ";
        assert_eq!(check(json!("x")), format!("{from_date}input is too short"));
        assert_eq!(
            check(json!("2026-01-01T25:00")),
            format!("{from_date}unexpected extra characters at the end of the input")
        );
        assert_eq!(check(json!("2026-02-30T10:00")), format!("{from_date}day value is outside expected range"));
        assert_eq!(check(json!("2026-0x-01T")), format!("{from_date}invalid character in month"));
        assert_eq!(check(json!("  2026-01-01T10:00")), format!("{from_date}invalid character in year"));
        assert_eq!(check(json!("99999999999999999999")), format!("{from_date}invalid date separator, expected `-`"));
        assert_eq!(check(json!("1e10")), format!("{from_date}input is too short"));
        assert_eq!(check(json!(true)), "datetime_type: Input should be a valid datetime");
        assert_eq!(
            check(json!(1e20)),
            "datetime_parsing: Input should be a valid datetime, dates after 9999 are not supported as unix timestamps"
        );
        assert_eq!(
            check(json!("0000-01-01")),
            "datetime_parsing: Input should be a valid datetime, year 0 is out of range"
        );
        assert_eq!(
            check(json!(-62_167_219_200_000_i64)),
            "datetime_parsing: Input should be a valid datetime, year 0 is out of range"
        );
    }
}
