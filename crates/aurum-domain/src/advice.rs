//! API contract types for `/api/advice` — port of the former Python backend's `app/schemas/advice.py`,
//! plus the pure rules of the former Python backend's `app/services/advice_service.py`.
//!
//! Personalized, read-only financial advice — a curated set of insights
//! computed from data that already exists (transactions, budgets, category
//! totals). Unlike `insights`' proactive alerts (urgent, shown as a banner
//! on every page), these are advisory notes shown only on the dedicated
//! Advice page — a mix of positive, neutral and cautionary tone, not just
//! warnings, and not something the user needs to act on immediately.
//!
//! Follows `crates/aurum-server/PORTING.md`: response DTOs derive
//! `Serialize` (field order = pydantic field order). No IO here — the
//! server gathers the inputs, these functions decide.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::category_rollup::CategoryContribution;
use crate::dashboard::DashboardSummary;
use crate::decimal::Decimal;
use crate::insights::{Params, py_round};
use crate::pyfmt::PyFloat;

/// `TRAILING_MONTHS`: the window a category's current spend is compared to.
pub const TRAILING_MONTHS: u32 = 3;
/// `RISING_CATEGORY_THRESHOLD_PERCENT`.
pub const RISING_CATEGORY_THRESHOLD_PERCENT: f64 = 25.0;
/// `SAVINGS_RATE_TREND_THRESHOLD_POINTS`.
pub const SAVINGS_RATE_TREND_THRESHOLD_POINTS: f64 = 5.0;

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

/// `Literal["positive", "neutral", "warning"]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum AdviceTone {
    Positive,
    Neutral,
    Warning,
}

/// One value of `dict[str, str | int | float]`. pydantic's smart union keeps
/// a Python `int` an int and a `float` a float (`100.0`, not `100`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AdviceParam {
    Str(String),
    Int(i64),
    Float(PyFloat),
}

impl From<&str> for AdviceParam {
    fn from(value: &str) -> Self {
        Self::Str(value.to_string())
    }
}

impl From<i64> for AdviceParam {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<f64> for AdviceParam {
    fn from(value: f64) -> Self {
        Self::Float(PyFloat(value))
    }
}

/// `AdviceItem`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AdviceItem {
    pub key: String,
    pub tone: AdviceTone,
    /// Frontend-translated by key — same key+params pattern as
    /// `FinancialAlert`, extended to allow string params (category names)
    /// alongside numbers.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub params: Params<AdviceParam>,
}

/// `AdviceResponse` — response of `GET /api/advice`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AdviceResponse {
    pub items: Vec<AdviceItem>,
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

/// `_category_expense_totals`: per-category sums in first-seen order (a
/// Python dict), which decides ties downstream.
pub fn category_totals(contributions: &[CategoryContribution]) -> Vec<(i32, Decimal)> {
    let mut totals: Vec<(i32, Decimal)> = Vec::new();
    let mut index: HashMap<i32, usize> = HashMap::new();
    for contribution in contributions {
        match index.get(&contribution.category_id) {
            Some(&i) => totals[i].1 = totals[i].1.add(&contribution.amount),
            None => {
                index.insert(contribution.category_id, totals.len());
                totals.push((contribution.category_id, Decimal::zero().add(&contribution.amount)));
            }
        }
    }
    totals
}

/// Adds one month's totals into the trailing-window sums.
pub fn accumulate(trailing: &mut HashMap<i32, Decimal>, month: &[(i32, Decimal)]) {
    for (cat_id, amount) in month {
        let entry = trailing.entry(*cat_id).or_insert_with(Decimal::zero);
        *entry = entry.add(amount);
    }
}

/// `_rising_category_advice`: the expense category furthest above its own
/// trailing-3-month average, if that's at least
/// `RISING_CATEGORY_THRESHOLD_PERCENT` higher. `names` maps the categories
/// that still exist to their name.
pub fn rising_category_advice(
    current: &[(i32, Decimal)],
    trailing: &HashMap<i32, Decimal>,
    names: &HashMap<i32, String>,
) -> Option<AdviceItem> {
    let months = Decimal::from(i64::from(TRAILING_MONTHS));
    let hundred = Decimal::from(100i64);
    let mut best: Option<(f64, i32, &Decimal, Decimal)> = None;
    for (cat_id, amount) in current {
        let average = trailing.get(cat_id).cloned().unwrap_or_else(Decimal::zero).div(&months)?;
        if average <= Decimal::zero() {
            continue;
        }
        let increase_percent = amount.sub(&average).div(&average)?.mul(&hundred).to_f64();
        if increase_percent >= RISING_CATEGORY_THRESHOLD_PERCENT && best.as_ref().is_none_or(|b| increase_percent > b.0)
        {
            best = Some((increase_percent, *cat_id, amount, average));
        }
    }

    let (increase_percent, cat_id, amount, average) = best?;
    let name = names.get(&cat_id)?;
    Some(AdviceItem {
        key: "rising_category".to_string(),
        tone: AdviceTone::Warning,
        params: Params::new()
            .with("category", name.as_str())
            .with("percent", py_round(increase_percent))
            .with("current", amount.to_f64())
            .with("average", average.to_f64()),
    })
}

/// `_unbudgeted_top_category_advice`: this month's highest-spending expense
/// category that has no budget (ties keep first-seen order, like Python's
/// stable `sorted(reverse=True)`).
pub fn unbudgeted_top_category_advice(
    current: &[(i32, Decimal)],
    budgeted: &HashSet<i32>,
    names: &HashMap<i32, String>,
) -> Option<AdviceItem> {
    let mut ranked: Vec<&(i32, Decimal)> = current.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));
    ranked.into_iter().filter(|(cat_id, _)| !budgeted.contains(cat_id)).find_map(|(cat_id, amount)| {
        let name = names.get(cat_id)?;
        Some(AdviceItem {
            key: "unbudgeted_top_category".to_string(),
            tone: AdviceTone::Neutral,
            params: Params::new().with("category", name.as_str()).with("amount", amount.to_f64()),
        })
    })
}

/// `float(net / real_income * 100)`, or `None` when `real_income <= 0`.
fn savings_rate(summary: &DashboardSummary) -> Option<f64> {
    if summary.real_income <= Decimal::zero() {
        return None;
    }
    Some(summary.net.div(&summary.real_income)?.mul(&Decimal::from(100i64)).to_f64())
}

/// `_savings_rate_trend_advice`: this month's savings rate (net /
/// real_income) vs. last month's, when the swing is large enough to be
/// worth mentioning.
pub fn savings_rate_trend_advice(current: &DashboardSummary, previous: &DashboardSummary) -> Option<AdviceItem> {
    let (current_rate, previous_rate) = (savings_rate(current)?, savings_rate(previous)?);
    let diff = current_rate - previous_rate;
    if diff.abs() < SAVINGS_RATE_TREND_THRESHOLD_POINTS {
        return None;
    }
    Some(AdviceItem {
        key: "savings_rate_trend".to_string(),
        tone: if diff > 0.0 { AdviceTone::Positive } else { AdviceTone::Warning },
        params: Params::new().with("rate", py_round(current_rate)).with("diff", py_round(diff)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn names() -> HashMap<i32, String> {
        HashMap::from([(1, "Food".to_string()), (2, "Fun".to_string()), (3, "Rent".to_string())])
    }

    fn summary(real_income: &str, net: &str) -> DashboardSummary {
        DashboardSummary {
            year: 2026,
            month: 1,
            real_income: dec(real_income),
            spent: Decimal::zero(),
            net: dec(net),
            transferred_out: Decimal::zero(),
            spending_by_category: Vec::new(),
        }
    }

    #[test]
    fn rising_category_picks_the_largest_increase() {
        let current = vec![(1, dec("130.00")), (2, dec("300.00")), (3, dec("10.00"))];
        let trailing = HashMap::from([(1, dec("300.00")), (2, dec("100.00")), (3, dec("90.00"))]);
        let item = rising_category_advice(&current, &trailing, &names()).unwrap();
        assert_eq!(
            serde_json::to_string(&item).unwrap(),
            r#"{"key":"rising_category","tone":"warning","params":{"category":"Fun","percent":800,"current":300.0,"average":33.333333333333336}}"#
        );
        // +20% is below the threshold; no history means no average.
        let quiet = vec![(1, dec("120.00")), (9, dec("1000"))];
        assert_eq!(rising_category_advice(&quiet, &trailing, &names()), None);
    }

    #[test]
    fn unbudgeted_top_category_skips_budgeted_and_keeps_tie_order() {
        let current = vec![(3, dec("50")), (1, dec("80")), (2, dec("80"))];
        let item = unbudgeted_top_category_advice(&current, &HashSet::new(), &names()).unwrap();
        assert_eq!(item.params.get("category"), Some(&AdviceParam::from("Food")));
        let item = unbudgeted_top_category_advice(&current, &HashSet::from([1]), &names()).unwrap();
        assert_eq!(item.params.get("category"), Some(&AdviceParam::from("Fun")));
        assert_eq!(unbudgeted_top_category_advice(&current, &HashSet::from([1, 2, 3]), &names()), None);
    }

    #[test]
    fn savings_rate_trend_needs_income_and_a_large_swing() {
        let item = savings_rate_trend_advice(&summary("1000", "300"), &summary("1000", "100")).unwrap();
        assert_eq!(serde_json::to_string(&item.params).unwrap(), r#"{"rate":30,"diff":20}"#);
        assert_eq!(item.tone, AdviceTone::Positive);
        let item = savings_rate_trend_advice(&summary("1000", "-25"), &summary("1000", "100")).unwrap();
        assert_eq!(item.tone, AdviceTone::Warning);
        assert_eq!(savings_rate_trend_advice(&summary("1000", "300"), &summary("1000", "260")), None);
        assert_eq!(savings_rate_trend_advice(&summary("0", "300"), &summary("1000", "100")), None);
    }

    #[test]
    fn params_deserialise_back_to_their_kinds() {
        let params: Params<AdviceParam> = serde_json::from_str(r#"{"a":"x","b":3,"c":3.0}"#).unwrap();
        assert_eq!(params.0[1].1, AdviceParam::Int(3));
        assert_eq!(params.0[2].1, AdviceParam::Float(PyFloat(3.0)));
    }
}
