//! Currency conversion — the pure half of `backend/app/services/fx_service.py`.
//!
//! Everything here is deterministic and IO-free (so mobile clients can reuse
//! it): ISO code normalisation, NBP date chunking, picking the publication
//! that answers a requested date, the cached-rate rules and the PLN-pivot
//! cross rate. Fetching from NBP and the `fx_rates` cache live in
//! `aurum-server`'s `services::fx` / `repo::fx`.
//!
//! NBP only ever receives currency codes and dates — never transaction
//! amounts, account names, descriptions or other user data.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde_json::Value;

use crate::decimal::Decimal;
use crate::pyfmt::py_float_repr;
use crate::text::{is_py_space, py_strip};

/// The earliest date NBP publishes rates for.
pub const NBP_EARLIEST_DATE: NaiveDate = match NaiveDate::from_ymd_opt(2002, 1, 2) {
    Some(d) => d,
    None => panic!("valid date"),
};
/// NBP's documented maximum range per request, in days.
pub const MAX_NBP_RANGE_DAYS: i64 = 93;
/// Table B is normally published weekly, so its holiday fallback window must
/// be longer than table A's business-day window.
pub const FALLBACK_DAYS: i64 = 14;
/// A rate cached for *today* from an earlier publication is provisional and
/// re-fetched once it is older than this.
pub const PROVISIONAL_RATE_TTL_SECS: i64 = 3600;
/// The pivot currency every NBP rate is quoted in.
pub const PIVOT: &str = "PLN";

/// An `HTTPException` raised by the FX service, as status + detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FxError {
    /// 422 — the request can never be answered (bad code, future date, ...).
    Unprocessable(String),
    /// 503 — the provider is unavailable or answered garbage; a cached
    /// nearby publication may still answer it.
    Unavailable(String),
}

impl FxError {
    pub fn status(&self) -> u16 {
        match self {
            FxError::Unprocessable(_) => 422,
            FxError::Unavailable(_) => 503,
        }
    }

    pub fn detail(&self) -> &str {
        match self {
            FxError::Unprocessable(d) | FxError::Unavailable(d) => d,
        }
    }
}

/// `_normalise_currency`: `currency.strip().upper()`, which must then be
/// three letters (Python's `str.isalpha`, so non-ASCII letters pass too).
pub fn normalise_currency(currency: &str) -> Result<String, FxError> {
    let value = py_strip(currency).to_uppercase();
    if value.chars().count() != 3 || !value.chars().all(char::is_alphabetic) {
        return Err(FxError::Unprocessable(format!("Unsupported currency code: {}", py_str_repr(currency))));
    }
    Ok(value)
}

/// Python's `repr(str)` (used by `{currency!r}` in error details).
pub fn py_str_repr(value: &str) -> String {
    let quote = if value.contains('\'') && !value.contains('"') { '"' } else { '\'' };
    let mut out = String::with_capacity(value.len() + 2);
    out.push(quote);
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            // Python escapes non-printable characters: controls, and every
            // separator except the plain space.
            c if c.is_control() || (c != ' ' && is_py_space(c)) || is_format_char(c) => {
                let code = c as u32;
                if code <= 0xff {
                    out.push_str(&format!("\\x{code:02x}"));
                } else if code <= 0xffff {
                    out.push_str(&format!("\\u{code:04x}"));
                } else {
                    out.push_str(&format!("\\U{code:08x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// The common Unicode format (Cf) characters Python treats as non-printable.
fn is_format_char(c: char) -> bool {
    matches!(c, '\u{ad}' | '\u{600}'..='\u{605}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}'
        | '\u{2060}'..='\u{2064}' | '\u{feff}')
}

/// Groups dates so each NBP range, including the fallback lookback, stays
/// within the provider's documented 93-day maximum (`_date_chunks`).
pub fn date_chunks(requested_dates: &BTreeSet<NaiveDate>) -> Vec<Vec<NaiveDate>> {
    let mut chunks: Vec<Vec<NaiveDate>> = Vec::new();
    for &requested in requested_dates {
        match chunks.last_mut() {
            Some(chunk) if (requested - chunk[0]).num_days() + FALLBACK_DAYS < MAX_NBP_RANGE_DAYS => {
                chunk.push(requested)
            }
            _ => chunks.push(vec![requested]),
        }
    }
    chunks
}

/// The first future date of a request (Python refuses the whole fetch with
/// it), formatted into the 422 detail.
pub fn future_date_error(requested_dates: &BTreeSet<NaiveDate>, today: NaiveDate) -> Option<FxError> {
    requested_dates.iter().find(|d| **d > today).map(|d| {
        FxError::Unprocessable(format!("Cannot resolve a historical FX rate for future date {}", d.format("%Y-%m-%d")))
    })
}

/// The provider date range `[start, end]` asked for one chunk, or the 422
/// raised when the chunk lies entirely before NBP's first publication.
pub fn chunk_range(chunk: &[NaiveDate], today: NaiveDate, currency: &str) -> Result<(NaiveDate, NaiveDate), FxError> {
    let (first, last) = (chunk[0], chunk[chunk.len() - 1]);
    let provider_end = last.min(today);
    let provider_start = NBP_EARLIEST_DATE.max(first.min(provider_end) - Duration::days(FALLBACK_DAYS));
    if provider_end < NBP_EARLIEST_DATE {
        return Err(FxError::Unprocessable(format!(
            "NBP has no {currency} rates before {}",
            NBP_EARLIEST_DATE.format("%Y-%m-%d")
        )));
    }
    Ok((provider_start, provider_end))
}

/// NBP's rates URL for one table (`a` first, then `b`).
pub fn nbp_url(base_url: &str, table: &str, currency: &str, start: NaiveDate, end: NaiveDate) -> String {
    format!(
        "{}/{table}/{}/{}/{}/",
        base_url.trim_end_matches('/'),
        currency.to_lowercase(),
        start.format("%Y-%m-%d"),
        end.format("%Y-%m-%d")
    )
}

/// Parses an NBP body into `(effectiveDate, mid)` pairs sorted like
/// Python's `sorted(tuples)`. `mid` goes through Python's
/// `Decimal(str(item["mid"]))`, i.e. the float's `repr`.
pub fn parse_nbp_rates(body: &Value) -> Result<Vec<(NaiveDate, Decimal)>, FxError> {
    let invalid = || FxError::Unavailable("FX provider returned an invalid response".into());
    // `for item in response.json()["rates"]`: an empty string/object
    // iterates as nothing, anything else that isn't a list of objects fails.
    let rates: &[Value] = match body.get("rates") {
        Some(Value::Array(items)) => items,
        Some(Value::String(s)) if s.is_empty() => &[],
        Some(Value::Object(m)) if m.is_empty() => &[],
        _ => return Err(invalid()),
    };
    let mut publications = Vec::with_capacity(rates.len());
    for item in rates {
        let date = item
            .get("effectiveDate")
            .and_then(Value::as_str)
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            .ok_or_else(invalid)?;
        let mid = match item.get("mid") {
            Some(Value::Number(n)) if n.is_i64() || n.is_u64() => n.to_string(),
            Some(Value::Number(n)) => py_float_repr(n.as_f64().ok_or_else(invalid)?),
            Some(Value::String(s)) => s.trim().to_string(),
            _ => return Err(invalid()),
        };
        let mid: Decimal = mid.parse().map_err(|_| invalid())?;
        publications.push((date, mid));
    }
    publications.sort();
    Ok(publications)
}

/// The publication answering each requested date of a chunk: the latest one
/// on or before `min(requested, today)`. Errors are 503s (an outage, which a
/// cached nearby rate may still cover).
pub fn pick_publications(
    chunk: &[NaiveDate],
    publications: &[(NaiveDate, Decimal)],
    today: NaiveDate,
    currency: &str,
) -> Result<Vec<(NaiveDate, NaiveDate, Decimal)>, FxError> {
    let mut resolved = Vec::with_capacity(chunk.len());
    for &requested in chunk {
        let lookup_date = requested.min(today);
        let Some((effective_date, rate)) = publications.iter().rev().find(|(d, _)| *d <= lookup_date) else {
            return Err(FxError::Unavailable(format!(
                "No {currency} rate available on or before {}",
                requested.format("%Y-%m-%d")
            )));
        };
        if *rate <= Decimal::zero() {
            return Err(FxError::Unavailable("FX provider returned a non-positive rate".into()));
        }
        resolved.push((requested, *effective_date, rate.clone()));
    }
    Ok(resolved)
}

/// One `fx_rates` row: one unit of `currency` was worth `rate_to_pln` PLN on
/// `requested_date`, from the publication of `effective_date`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct FxRateRow {
    pub currency: String,
    pub requested_date: NaiveDate,
    pub effective_date: NaiveDate,
    pub rate_to_pln: Decimal,
    pub fetched_at: DateTime<Utc>,
}

impl FxRateRow {
    /// A rate stored for today from an earlier publication, older than the
    /// TTL: NBP may have published today's rate since, so it isn't reused.
    pub fn is_expired_provisional(&self, today: NaiveDate, now: DateTime<Utc>) -> bool {
        self.requested_date == today
            && self.effective_date < self.requested_date
            && self.fetched_at < now - Duration::seconds(PROVISIONAL_RATE_TTL_SECS)
    }
}

/// Cached rates usable as-is, keyed by `(currency, requested_date)`.
pub fn usable_cached_rates(
    rows: &[FxRateRow],
    today: NaiveDate,
    now: DateTime<Utc>,
) -> BTreeMap<(String, NaiveDate), Decimal> {
    rows.iter()
        .filter(|row| !row.is_expired_provisional(today, now))
        .map(|row| ((row.currency.clone(), row.requested_date), row.rate_to_pln.clone()))
        .collect()
}

/// A publication already cached for a nearby earlier date — the outage
/// fallback (`cached_fallback`): the latest by `(effective_date,
/// fetched_at)` within the fallback window; the first such row wins a tie.
pub fn cached_fallback(rows: &[FxRateRow], currency: &str, requested: NaiveDate) -> Option<Decimal> {
    let window_start = requested - Duration::days(FALLBACK_DAYS);
    let mut best: Option<&FxRateRow> = None;
    for row in rows {
        if row.currency == currency && window_start <= row.effective_date && row.effective_date <= requested {
            let better = match best {
                None => true,
                Some(b) => (row.effective_date, row.fetched_at) > (b.effective_date, b.fetched_at),
            };
            if better {
                best = Some(row);
            }
        }
    }
    best.map(|row| row.rate_to_pln.clone())
}

/// An amount to convert: `key` identifies it in the result.
#[derive(Debug, Clone, PartialEq)]
pub struct MoneyAmount<K> {
    pub key: K,
    pub amount: Decimal,
    pub currency: String,
    pub value_date: NaiveDate,
}

impl<K> MoneyAmount<K> {
    pub fn new(key: K, amount: Decimal, currency: impl Into<String>, value_date: NaiveDate) -> Self {
        Self { key, amount, currency: currency.into(), value_date }
    }
}

/// Which `(currency, dates)` need a PLN rate to convert `items` (already
/// normalised) into `target`.
///
/// NOTE(parity): like Python, the target's rates are requested for every
/// item, even for items already in the target currency (which are then
/// passed through unconverted). That makes a same-currency conversion hit
/// NBP / the cache for nothing — a known Python quirk kept deliberately so
/// both servers issue the same provider requests and fail the same way.
pub fn required_rates<K>(items: &[MoneyAmount<K>], target: &str) -> BTreeMap<String, BTreeSet<NaiveDate>> {
    let mut needed: BTreeMap<String, BTreeSet<NaiveDate>> = BTreeMap::new();
    for item in items {
        if item.currency != PIVOT {
            needed.entry(item.currency.clone()).or_default().insert(item.value_date);
        }
        if target != PIVOT {
            needed.entry(target.to_string()).or_default().insert(item.value_date);
        }
    }
    needed
}

/// `amount * source_to_pln / target_to_pln` under Python's decimal context;
/// PLN legs use `Decimal("1")`. Items already in `target` are returned
/// untouched. `rate` looks up a cached/fetched PLN rate.
pub fn convert_one(
    amount: &Decimal,
    currency: &str,
    target: &str,
    value_date: NaiveDate,
    rate: impl Fn(&str, NaiveDate) -> Option<Decimal>,
) -> Option<Decimal> {
    if currency == target {
        return Some(amount.clone());
    }
    let one = || Decimal::from(1i64);
    let source_to_pln = if currency == PIVOT { one() } else { rate(currency, value_date)? };
    let target_to_pln = if target == PIVOT { one() } else { rate(target, value_date)? };
    amount.mul(&source_to_pln).div(&target_to_pln)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn dec(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    #[test]
    fn currency_normalisation_and_repr() {
        assert_eq!(normalise_currency(" usd ").unwrap(), "USD");
        assert_eq!(normalise_currency("US1").unwrap_err().detail(), "Unsupported currency code: 'US1'");
        assert_eq!(py_str_repr("it's"), "\"it's\"");
        assert_eq!(py_str_repr("a\u{1}\n"), "'a\\x01\\n'");
    }

    #[test]
    fn chunks_respect_the_93_day_limit() {
        // 77 + 14 < 93 stays in the chunk, 79 + 14 = 93 starts a new one.
        let dates: BTreeSet<_> = [day(2026, 1, 1), day(2026, 3, 19), day(2026, 3, 21), day(2026, 6, 1)].into();
        let chunks = date_chunks(&dates);
        assert_eq!(chunks, vec![vec![day(2026, 1, 1), day(2026, 3, 19)], vec![day(2026, 3, 21), day(2026, 6, 1)]]);
    }

    #[test]
    fn publications_follow_python_parsing() {
        let body = serde_json::json!({"rates": [
            {"effectiveDate": "2026-01-16", "mid": 4.0},
            {"effectiveDate": "2026-01-15", "mid": 3}
        ]});
        let publications = parse_nbp_rates(&body).unwrap();
        assert_eq!(publications[0], (day(2026, 1, 15), dec("3")));
        assert_eq!(publications[1].1.to_string(), "4.0");
        let picked = pick_publications(&[day(2026, 1, 18)], &publications, day(2026, 1, 20), "USD").unwrap();
        assert_eq!(picked[0].1, day(2026, 1, 16));
        assert!(parse_nbp_rates(&serde_json::json!({"rates": [{"mid": 1}]})).is_err());
    }

    #[test]
    fn same_currency_still_requests_target_rates() {
        let items = vec![MoneyAmount::new(0, dec("1"), "USD", day(2026, 1, 15))];
        let needed = required_rates(&items, "USD");
        assert_eq!(needed.get("USD").map(BTreeSet::len), Some(1));
        assert!(required_rates(&items, PIVOT).contains_key("USD"));
    }

    #[test]
    fn cross_rates_use_python_arithmetic() {
        let rate = |c: &str, _| match c {
            "USD" => Some(dec("4.000000000000000000")),
            "UAH" => Some(dec("0.1")),
            _ => None,
        };
        let d = day(2026, 1, 15);
        assert_eq!(convert_one(&dec("100.00"), "USD", "PLN", d, rate).unwrap().to_string(), "400.00000000000000000000");
        assert_eq!(convert_one(&dec("1000.00"), "UAH", "PLN", d, rate).unwrap().to_string(), "100.000");
        assert_eq!(convert_one(&dec("7.00"), "PLN", "PLN", d, rate).unwrap().to_string(), "7.00");
    }

    #[test]
    fn fallback_prefers_latest_publication() {
        let now = Utc::now();
        let row = |eff: NaiveDate, rate: &str| FxRateRow {
            currency: "UAH".into(),
            requested_date: eff,
            effective_date: eff,
            rate_to_pln: dec(rate),
            fetched_at: now,
        };
        let rows = vec![row(day(2026, 1, 14), "0.08"), row(day(2026, 1, 16), "0.083")];
        assert_eq!(cached_fallback(&rows, "UAH", day(2026, 1, 17)), Some(dec("0.083")));
        assert_eq!(cached_fallback(&rows, "UAH", day(2026, 1, 13)), None);
    }
}
