//! API contract types for `/api/insights` — port of the former Python backend's `app/schemas/insights.py`,
//! plus the pure rules of the former Python backend's `app/services/insights_service.py`
//! (proactive early-warning checks, computed from data that already exists).
//!
//! Five signals: sustained negative monthly cash flow, a sustained net-worth
//! decline, one or more over-budget categories, too much capital in
//! medium/high risk tiers (the "80% at zero risk, 20% at most exposed"
//! rule), and cash sitting idle in a depository account. The first two only
//! look at fully-elapsed calendar months, so a same-month false alarm (rent
//! already paid, salary not landed yet) never fires — their thresholds, the
//! risk-allocation percentage and the idle-cash amount/days are all
//! user-configurable (`AppSettings`). The budget check deliberately looks
//! at the CURRENT, still-in-progress month.
//!
//! Follows `crates/aurum-server/PORTING.md`: response DTOs derive
//! `Serialize` (field order = pydantic field order). No IO here — the
//! server gathers the inputs, these functions decide.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::marker::PhantomData;

use chrono::{Datelike, NaiveDate};
use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::decimal::Decimal;
use crate::enums::AccountType;
use crate::net_worth::{NetWorthPoint, RiskLevelSummary};

/// `MAX_LOOKBACK_MONTHS`: how far back the negative-cash-flow streak looks.
pub const MAX_LOOKBACK_MONTHS: usize = 24;

/// Accounts where a big, untouched balance means money isn't working — a
/// credit card balance isn't "your cash", an investment account is already
/// invested, and OTHER is too ambiguous to guess at.
pub const IDLE_CASH_ACCOUNT_TYPES: [AccountType; 4] =
    [AccountType::Checking, AccountType::DebitCard, AccountType::Savings, AccountType::Cash];

/// The only severity the alerts use today.
pub const SEVERITY_WARNING: &str = "warning";

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

/// A Python `dict[str, V]` param bag: insertion-ordered, serialised as a
/// JSON object in that order (pydantic keeps the dict's order). Shared with
/// `advice::AdviceItem`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Params<V>(pub Vec<(String, V)>);

impl<V> Params<V> {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// `params[key] = value` for a fresh key (callers never repeat a key).
    pub fn with(mut self, key: &str, value: impl Into<V>) -> Self {
        self.0.push((key.to_string(), value.into()));
        self
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

impl<V: Serialize> Serialize for Params<V> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (key, value) in &self.0 {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl<'de, V: Deserialize<'de>> Deserialize<'de> for Params<V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ParamsVisitor<V>(PhantomData<V>);

        impl<'de, V: Deserialize<'de>> Visitor<'de> for ParamsVisitor<V> {
            type Value = Params<V>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON object of params")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                let mut entries = Vec::new();
                while let Some((key, value)) = access.next_entry::<String, V>()? {
                    entries.push((key, value));
                }
                Ok(Params(entries))
            }
        }

        deserializer.deserialize_map(ParamsVisitor(PhantomData))
    }
}

/// `FinancialAlert`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FinancialAlert {
    pub key: String,
    pub severity: String,
    /// Machine-readable params for the frontend to interpolate into its own
    /// localized message template (keyed by `key`) — keeps alert copy out of
    /// the backend so it can be translated without an API change.
    #[cfg_attr(feature = "openapi", schema(value_type = HashMap<String, i64>))]
    pub params: Params<i64>,
}

impl FinancialAlert {
    fn warning(key: &str, params: Params<i64>) -> Self {
        Self { key: key.to_string(), severity: SEVERITY_WARNING.to_string(), params }
    }
}

/// `AlertsResponse` — response of `GET /api/insights/alerts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AlertsResponse {
    pub alerts: Vec<FinancialAlert>,
}

// ---------------------------------------------------------------------------
// Python numeric helpers
// ---------------------------------------------------------------------------

/// `(year - 1, 12) if month == 1 else (year, month - 1)`.
pub fn previous_month(year: i32, month: u32) -> (i32, u32) {
    if month == 1 { (year - 1, 12) } else { (year, month - 1) }
}

/// Python's built-in `round(x)` on a float: round half to even, to an int.
pub fn py_round(value: f64) -> i64 {
    value.round_ties_even() as i64
}

/// Python's built-in `sum()` over floats (start `0`). CPython ≥ 3.12 adds
/// floats with Neumaier compensation, so a plain fold can differ in the
/// last bit for three or more terms.
pub fn py_sum_floats(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut total = 0.0_f64;
    let mut compensation = 0.0_f64;
    for x in values {
        let t = total + x;
        if total.abs() >= x.abs() {
            compensation += (total - t) + x;
        } else {
            compensation += (x - t) + total;
        }
        total = t;
    }
    if compensation != 0.0 && compensation.is_finite() { total + compensation } else { total }
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

/// `_net_worth_decline_streak`: how many consecutive month-end values
/// (counting back from the last fully-elapsed month) were lower than the
/// month before.
pub fn net_worth_decline_streak(series: &[NetWorthPoint], today: NaiveDate) -> usize {
    // Keep the last point seen for each (year, month) — `series` is ordered
    // ascending by date, so that's the month-end value — and drop the
    // current month, which is always partial.
    let mut month_end: BTreeMap<(i32, u32), &Decimal> = BTreeMap::new();
    for point in series {
        let key = (point.date.year(), point.date.month());
        if key == (today.year(), today.month()) {
            continue;
        }
        month_end.insert(key, &point.value);
    }
    let ordered: Vec<&Decimal> = month_end.into_values().collect();
    if ordered.len() < 2 {
        return 0;
    }
    ordered.windows(2).rev().take_while(|pair| pair[1] < pair[0]).count()
}

/// `sum(tier.percent for tier in risk_levels if tier.risk_level != "low")`.
pub fn risky_percent(risk_levels: &[RiskLevelSummary]) -> f64 {
    py_sum_floats(risk_levels.iter().filter(|tier| tier.risk_level != "low").map(|tier| tier.percent.0))
}

/// One eligible (non-archived, depository) account for the idle-cash check.
#[derive(Debug, Clone, PartialEq)]
pub struct IdleCashCandidate {
    /// balance converted into the display currency
    pub converted_balance: Decimal,
    /// the most recent transaction date touching the account (either side)
    pub last_activity: Option<NaiveDate>,
}

/// `_idle_cash_account_count`: accounts holding at least `threshold_amount`
/// whose last activity is on or before `cutoff` (no activity at all counts
/// as idle).
pub fn idle_cash_count(candidates: &[IdleCashCandidate], threshold_amount: &Decimal, cutoff: NaiveDate) -> usize {
    candidates
        .iter()
        .filter(|c| &c.converted_balance >= threshold_amount && c.last_activity.unwrap_or(cutoff) <= cutoff)
        .count()
}

/// Everything `get_financial_alerts` measured, already reduced to numbers.
#[derive(Debug, Clone, PartialEq)]
pub struct AlertInputs {
    pub negative_cash_flow_streak: usize,
    pub net_worth_decline_streak: usize,
    pub risky_percent: f64,
    pub over_budget_count: usize,
    pub idle_cash_count: usize,
}

/// The user-configurable thresholds (`AppSettings`).
#[derive(Debug, Clone, PartialEq)]
pub struct AlertThresholds {
    pub negative_cash_flow_months: i32,
    pub net_worth_decline_months: i32,
    pub risky_allocation_percent: i32,
    pub idle_cash_days: i32,
}

/// Which alerts fire, in Python's order.
pub fn financial_alerts(inputs: &AlertInputs, thresholds: &AlertThresholds) -> AlertsResponse {
    let mut alerts = Vec::new();
    let as_int = |n: usize| n as i64;

    if as_int(inputs.negative_cash_flow_streak) >= i64::from(thresholds.negative_cash_flow_months) {
        alerts.push(FinancialAlert::warning(
            "negative_cash_flow_streak",
            Params::new().with("months", as_int(inputs.negative_cash_flow_streak)),
        ));
    }
    if as_int(inputs.net_worth_decline_streak) >= i64::from(thresholds.net_worth_decline_months) {
        alerts.push(FinancialAlert::warning(
            "net_worth_decline_streak",
            Params::new().with("months", as_int(inputs.net_worth_decline_streak)),
        ));
    }
    // float > int compares exactly in Python; an i32 is exact as f64.
    if inputs.risky_percent > f64::from(thresholds.risky_allocation_percent) {
        alerts.push(FinancialAlert::warning(
            "risky_allocation_exceeded",
            Params::new()
                .with("percent", py_round(inputs.risky_percent))
                .with("threshold", i64::from(thresholds.risky_allocation_percent)),
        ));
    }
    if inputs.over_budget_count > 0 {
        alerts.push(FinancialAlert::warning(
            "budget_exceeded",
            Params::new().with("count", as_int(inputs.over_budget_count)),
        ));
    }
    if inputs.idle_cash_count > 0 {
        alerts.push(FinancialAlert::warning(
            "idle_cash",
            Params::new()
                .with("count", as_int(inputs.idle_cash_count))
                .with("days", i64::from(thresholds.idle_cash_days)),
        ));
    }
    AlertsResponse { alerts }
}

/// Converted balances and last-activity dates joined per eligible account.
pub fn idle_cash_candidates(
    account_ids: &[i32],
    converted: &HashMap<i32, Decimal>,
    last_activity: &HashMap<i32, NaiveDate>,
) -> Vec<IdleCashCandidate> {
    account_ids
        .iter()
        .map(|id| IdleCashCandidate {
            converted_balance: converted.get(id).cloned().unwrap_or_else(Decimal::zero),
            last_activity: last_activity.get(id).copied(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pyfmt::PyFloat;

    fn dec(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn point(y: i32, m: u32, d: u32, v: &str) -> NetWorthPoint {
        NetWorthPoint { date: day(y, m, d), value: dec(v) }
    }

    #[test]
    fn rounding_and_sums_follow_python() {
        assert_eq!(py_round(2.5), 2);
        assert_eq!(py_round(3.5), 4);
        assert_eq!(py_round(-0.4), 0);
        assert_eq!(py_sum_floats([]), 0.0);
        // Neumaier: plain left-to-right gives 0.0 here, Python's sum 1.0.
        assert_eq!(py_sum_floats([1e100, 1.0, -1e100]), 1.0);
    }

    #[test]
    fn decline_streak_uses_month_ends_and_skips_current_month() {
        let today = day(2026, 5, 10);
        let series = [
            point(2026, 1, 31, "100"),
            point(2026, 2, 1, "500"),
            point(2026, 2, 28, "90"),
            point(2026, 3, 31, "80"),
            point(2026, 4, 30, "70"),
            point(2026, 5, 1, "10"),
        ];
        assert_eq!(net_worth_decline_streak(&series, today), 3);
        assert_eq!(net_worth_decline_streak(&series[..1], today), 0);
        let flat = [point(2026, 3, 31, "80"), point(2026, 4, 30, "80")];
        assert_eq!(net_worth_decline_streak(&flat, today), 0);
    }

    #[test]
    fn idle_cash_counts_large_stale_balances() {
        let cutoff = day(2026, 1, 1);
        let candidates = [
            IdleCashCandidate { converted_balance: dec("5000"), last_activity: None },
            IdleCashCandidate { converted_balance: dec("5000"), last_activity: Some(cutoff) },
            IdleCashCandidate { converted_balance: dec("5000"), last_activity: Some(day(2026, 1, 2)) },
            IdleCashCandidate { converted_balance: dec("4999.99"), last_activity: None },
        ];
        assert_eq!(idle_cash_count(&candidates, &dec("5000.00"), cutoff), 2);
    }

    #[test]
    fn alerts_serialise_in_python_order() {
        let inputs = AlertInputs {
            negative_cash_flow_streak: 3,
            net_worth_decline_streak: 0,
            risky_percent: 20.5,
            over_budget_count: 1,
            idle_cash_count: 2,
        };
        let thresholds = AlertThresholds {
            negative_cash_flow_months: 3,
            net_worth_decline_months: 3,
            risky_allocation_percent: 20,
            idle_cash_days: 90,
        };
        let json = serde_json::to_string(&financial_alerts(&inputs, &thresholds)).unwrap();
        assert_eq!(
            json,
            r#"{"alerts":[{"key":"negative_cash_flow_streak","severity":"warning","params":{"months":3}},{"key":"risky_allocation_exceeded","severity":"warning","params":{"percent":20,"threshold":20}},{"key":"budget_exceeded","severity":"warning","params":{"count":1}},{"key":"idle_cash","severity":"warning","params":{"count":2,"days":90}}]}"#
        );
        let back: AlertsResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.alerts[1].params.get("threshold"), Some(&20));
    }

    #[test]
    fn risky_percent_skips_low_tier() {
        let tier = |level: &str, percent: f64| RiskLevelSummary {
            risk_level: level.to_string(),
            label: String::new(),
            color: String::new(),
            total_value: Decimal::zero(),
            percent: PyFloat(percent),
            items: Vec::new(),
        };
        assert_eq!(risky_percent(&[tier("low", 70.0), tier("medium", 20.0), tier("high", 10.0)]), 30.0);
    }
}
