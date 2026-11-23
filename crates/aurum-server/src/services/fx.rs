//! Currency conversion behind one batch interface — port of
//! the former Python backend's `app/services/fx_service.py`.
//!
//! Callers provide native amounts and dates; this module alone knows how to
//! normalise ISO codes, resolve NBP business-day fallbacks, persist rates
//! and compute PLN-pivot cross rates (the pure parts live in
//! `aurum_domain::fx`). NBP only receives currency codes and dates — never
//! transaction amounts, account names, descriptions or other user data.
//!
//! Public API for other services: [`convert_amounts`] (with
//! [`MoneyAmount`]); errors are the same `HTTPException`s Python raised
//! (422 for an impossible request, 503 when NBP is unavailable and no cached
//! rate covers it).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::Hash;
use std::sync::OnceLock;
use std::time::Duration;

use aurum_domain::Decimal;
use aurum_domain::fx::{
    FxError, FxRateRow, PIVOT, cached_fallback, chunk_range, convert_one, date_chunks, future_date_error, nbp_url,
    normalise_currency, parse_nbp_rates, pick_publications, required_rates, usable_cached_rates,
};
use axum::http::StatusCode;
use chrono::{Local, NaiveDate, Utc};

pub use aurum_domain::fx::MoneyAmount;

use crate::error::{ApiError, ApiResult};
use crate::repo;
use crate::repo::fx::NewFxRate;
use crate::state::AppState;

/// Python's per-call `httpx.AsyncClient(timeout=12)`.
const NBP_TIMEOUT: Duration = Duration::from_secs(12);

impl From<FxError> for ApiError {
    fn from(e: FxError) -> Self {
        let status = StatusCode::from_u16(e.status()).unwrap_or(StatusCode::SERVICE_UNAVAILABLE);
        ApiError::http(status, e.detail())
    }
}

/// httpx doesn't follow redirects by default (a 3xx is "FX provider
/// returned HTTP 3xx"), reqwest does — so NBP gets its own client.
fn nbp_client(state: &AppState) -> reqwest::Client {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(NBP_TIMEOUT)
                .build()
                .ok()
        })
        .clone()
        .unwrap_or_else(|| state.http.clone())
}

/// `_fetch_nbp_currency`: the publication answering each requested date,
/// as `requested → (effective_date, rate)`, in date order.
async fn fetch_nbp_currency(
    state: &AppState,
    currency: &str,
    requested_dates: &BTreeSet<NaiveDate>,
) -> Result<Vec<(NaiveDate, NaiveDate, Decimal)>, FxError> {
    let today = Local::now().date_naive();
    if let Some(e) = future_date_error(requested_dates, today) {
        return Err(e);
    }
    let unavailable = || FxError::Unavailable("FX provider is temporarily unavailable".into());
    let client = nbp_client(state);
    let mut resolved = Vec::new();
    for chunk in date_chunks(requested_dates) {
        let (start, end) = chunk_range(&chunk, today, currency)?;
        let mut response = None;
        for table in ["a", "b"] {
            let url = format!("{}?format=json", nbp_url(&state.settings.nbp_base_url, table, currency, start, end));
            let candidate = client
                .get(url)
                .header("Accept", "application/json")
                .header("User-Agent", "Aurum/1.2")
                .timeout(NBP_TIMEOUT)
                .send()
                .await
                .map_err(|_| unavailable())?;
            if candidate.status() != reqwest::StatusCode::NOT_FOUND {
                response = Some(candidate);
                break;
            }
        }
        let Some(response) = response else {
            return Err(FxError::Unprocessable(format!("Currency {currency} is not available from NBP")));
        };
        let status = response.status();
        // httpx reads the whole body inside `client.get`, so a broken body is
        // a transport error there too.
        let body = response.bytes().await.map_err(|_| unavailable())?;
        if !status.is_success() {
            return Err(FxError::Unavailable(format!("FX provider returned HTTP {}", status.as_u16())));
        }
        let publications = serde_json::from_slice(&body)
            .map_err(|_| FxError::Unavailable("FX provider returned an invalid response".into()))
            .and_then(|json| parse_nbp_rates(&json))?;
        resolved.extend(pick_publications(&chunk, &publications, today, currency)?);
    }
    Ok(resolved)
}

/// `_rates_to_pln`: PLN rates for every `(currency, date)` in `needed`,
/// from the cache when possible, else from NBP (persisted for next time).
async fn rates_to_pln(
    state: &AppState,
    needed: &BTreeMap<String, BTreeSet<NaiveDate>>,
) -> ApiResult<BTreeMap<(String, NaiveDate), Decimal>> {
    let currencies: Vec<String> = needed.keys().filter(|c| c.as_str() != PIVOT).cloned().collect();
    if currencies.is_empty() {
        return Ok(BTreeMap::new());
    }
    let requested_dates: BTreeSet<NaiveDate> = currencies.iter().flat_map(|c| needed[c].iter().copied()).collect();
    let (Some(&first), Some(&latest)) = (requested_dates.first(), requested_dates.last()) else {
        return Ok(BTreeMap::new());
    };
    let earliest = first - chrono::Duration::days(aurum_domain::fx::FALLBACK_DAYS);
    let dates: Vec<NaiveDate> = requested_dates.iter().copied().collect();
    let cached_rows: Vec<FxRateRow> = repo::fx::cached(&state.pool, &currencies, &dates, earliest, latest).await?;

    let today = Local::now().date_naive();
    let mut rates = usable_cached_rates(&cached_rows, today, Utc::now());
    let mut new_rows = Vec::new();
    // `currencies` is sorted (BTreeMap keys), like Python's `sorted()`.
    for currency in &currencies {
        let missing: BTreeSet<NaiveDate> =
            needed[currency].iter().copied().filter(|d| !rates.contains_key(&(currency.clone(), *d))).collect();
        if missing.is_empty() {
            continue;
        }
        match fetch_nbp_currency(state, currency, &missing).await {
            Ok(fetched) => {
                for (requested, effective, rate) in fetched {
                    rates.insert((currency.clone(), requested), rate.clone());
                    new_rows.push(NewFxRate {
                        currency: currency.clone(),
                        requested_date: requested,
                        effective_date: effective,
                        rate_to_pln: rate,
                    });
                }
            }
            // A publication already cached for a nearby earlier date is a
            // safe outage fallback. It is intentionally not persisted under
            // the requested date, so today's preliminary value can still be
            // refreshed once NBP is available again.
            Err(e @ FxError::Unavailable(_)) => {
                let mut unresolved = false;
                for requested in &missing {
                    match cached_fallback(&cached_rows, currency, *requested) {
                        Some(rate) => {
                            rates.insert((currency.clone(), *requested), rate);
                        }
                        None => unresolved = true,
                    }
                }
                if unresolved {
                    return Err(e.into());
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
    if !new_rows.is_empty() {
        repo::fx::upsert(&state.pool, &new_rows).await?;
    }
    Ok(rates)
}

/// Converts every item atomically into `target_currency`.
///
/// The result is keyed exactly like the input. A missing rate fails the
/// whole batch; callers never receive a partially converted financial
/// total. Amounts already in the target currency come back untouched;
/// converted ones carry Python's decimal arithmetic (callers quantize).
pub async fn convert_amounts<K: Clone + Eq + Hash>(
    state: &AppState,
    amounts: &[MoneyAmount<K>],
    target_currency: &str,
) -> ApiResult<HashMap<K, Decimal>> {
    let target = normalise_currency(target_currency)?;
    let mut normalised = Vec::with_capacity(amounts.len());
    for item in amounts {
        normalised.push(MoneyAmount {
            key: item.key.clone(),
            amount: item.amount.clone(),
            currency: normalise_currency(&item.currency)?,
            value_date: item.value_date,
        });
    }
    let needed = required_rates(&normalised, &target);
    let rates = rates_to_pln(state, &needed).await?;
    let mut converted = HashMap::with_capacity(normalised.len());
    for item in normalised {
        let value = convert_one(&item.amount, &item.currency, &target, item.value_date, |c, d| {
            rates.get(&(c.to_string(), d)).cloned()
        })
        .ok_or_else(|| ApiError::internal(anyhow::anyhow!("no {} rate for {}", item.currency, item.value_date)))?;
        converted.insert(item.key, value);
    }
    Ok(converted)
}
