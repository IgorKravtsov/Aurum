//! API contract types for `/api/budgets` — port of `backend/app/schemas/budget.py`,
//! plus the pure half of `backend/app/services/budget_service.py`
//! ([`budget_status`]: each limit compared against a month's spend).
//!
//! Follows `crates/aurum-server/PORTING.md`: response DTOs derive
//! `Serialize` (field order = pydantic field order, inherited fields first),
//! request DTOs implement `validation::FromInput`.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::category_rollup::CategoryContribution;
use crate::decimal::Decimal;
use crate::pyfmt::PyFloat;
use crate::validation::{FromInput, Val, ValidationErrors};

// ---------------------------------------------------------------------------
// Request DTOs
// ---------------------------------------------------------------------------

/// `BudgetCreate` — body of `POST /api/budgets`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BudgetCreate {
    pub category_id: i64,
    pub monthly_limit: Decimal,
}

impl FromInput for BudgetCreate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let category_id = o.required("category_id", |v| v.int());
        // monthly_limit: Decimal = Field(gt=0) — no max_digits: an
        // over-long value fails in the database (500), like Python.
        let monthly_limit = o.required("monthly_limit", |v| v.decimal()?.gt("0"));
        o.finish()?;
        Ok(Self { category_id: category_id.value(), monthly_limit: monthly_limit.value() })
    }
}

/// `BudgetUpdate` — body of `PATCH /api/budgets/{budget_id}` (the limit is
/// required, it's the only thing a budget has to change).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BudgetUpdate {
    pub monthly_limit: Decimal,
}

impl FromInput for BudgetUpdate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let monthly_limit = o.required("monthly_limit", |v| v.decimal()?.gt("0"));
        o.finish()?;
        Ok(Self { monthly_limit: monthly_limit.value() })
    }
}

/// Path parameters of `PATCH/DELETE /api/budgets/{budget_id}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetPath {
    pub budget_id: i64,
}

impl FromInput for BudgetPath {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let budget_id = o.required("budget_id", |v| v.int());
        o.finish()?;
        Ok(Self { budget_id: budget_id.value() })
    }
}

/// Query of `GET /api/budgets/status`. `None` = "not given": the server
/// fills in today's year/month (`Query(default_factory=...)` — the default
/// itself is never validated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BudgetStatusQuery {
    pub year: Option<i64>,
    pub month: Option<i64>,
}

impl FromInput for BudgetStatusQuery {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let year = o.optional("year", |v| v.int()?.ge(2000)?.le(2100));
        let month = o.optional("month", |v| v.int()?.ge(1)?.le(12));
        o.finish()?;
        Ok(Self { year: year.value(), month: month.value() })
    }
}

// ---------------------------------------------------------------------------
// Response DTOs
// ---------------------------------------------------------------------------

/// `BudgetRead`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BudgetRead {
    pub id: i32,
    pub category_id: i32,
    pub category_name: String,
    pub category_color: String,
    pub category_icon: Option<String>,
    pub monthly_limit: Decimal,
}

/// `BudgetStatus` — one budgeted category's actual spend for a given month,
/// compared against its limit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BudgetStatus {
    pub budget_id: i32,
    pub category_id: i32,
    pub category_name: String,
    pub category_color: String,
    pub category_icon: Option<String>,
    pub monthly_limit: Decimal,
    pub spent: Decimal,
    pub remaining: Decimal,
    // Can exceed 100 — the UI clamps the progress bar fill, not this number,
    // so "spent 150% of budget" is still visible in the raw value.
    pub percent: PyFloat,
    pub is_over_budget: bool,
}

/// `BudgetStatusResponse`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BudgetStatusResponse {
    pub year: i32,
    pub month: u32,
    pub items: Vec<BudgetStatus>,
}

// ---------------------------------------------------------------------------
// Pure logic
// ---------------------------------------------------------------------------

/// `_month_bounds`: first and last day of the month (`None` for a month
/// chrono can't represent).
pub fn month_bounds(year: i32, month: u32) -> Option<(NaiveDate, NaiveDate)> {
    let start = NaiveDate::from_ymd_opt(year, month, 1)?;
    let (next_year, next_month) = if month == 12 { (year.checked_add(1)?, 1) } else { (year, month + 1) };
    let end = NaiveDate::from_ymd_opt(next_year, next_month, 1)?.pred_opt()?;
    debug_assert_eq!(end.month(), month);
    Some((start, end))
}

/// `float(part / whole * 100) if whole else 0.0` — the progress percentage
/// budgets and goals report (Python's 28-digit decimal context, then a
/// correctly rounded float).
pub fn percent_of(part: &Decimal, whole: &Decimal) -> PyFloat {
    match part.div(whole) {
        Some(ratio) => PyFloat(ratio.mul(&Decimal::from(100i64)).to_f64()),
        None => PyFloat(0.0),
    }
}

/// [`budget_status`]'s view of one budget row (`list_budgets` order).
#[derive(Debug, Clone, PartialEq)]
pub struct BudgetWithCategory {
    pub id: i32,
    pub category_id: i32,
    pub category_name: String,
    pub category_color: String,
    pub category_icon: Option<String>,
    pub monthly_limit: Decimal,
}

impl From<BudgetWithCategory> for BudgetRead {
    fn from(b: BudgetWithCategory) -> Self {
        Self {
            id: b.id,
            category_id: b.category_id,
            category_name: b.category_name,
            category_color: b.category_color,
            category_icon: b.category_icon,
            monthly_limit: b.monthly_limit,
        }
    }
}

/// The in-memory half of `get_budget_status`.
///
/// A budget on a top-level category covers its subcategories too — the
/// same rollup the Dashboard breakdown and the category report already do
/// (coalesce(parent_id, id) in dashboard_service/reports_service).
/// Counting only exact category_id matches meant a month could read as 900
/// spent on the Dashboard and 0 against its own budget. A subcategory that
/// has a budget of its own still tracks its own spending: both bars move,
/// each against its own limit.
///
/// `children` are the `(child_id, parent_id)` rows of every category whose
/// parent is budgeted, in query order; `contributions` are the month's
/// converted expense contributions, in seam order (sums are added in the
/// same order as Python, which matters for long converted decimals).
pub fn budget_status(
    budgets: &[BudgetWithCategory],
    children: &[(i32, i32)],
    contributions: &[CategoryContribution],
) -> Vec<BudgetStatus> {
    let mut children_by_parent: HashMap<i32, Vec<i32>> = HashMap::new();
    for (child_id, parent_id) in children {
        children_by_parent.entry(*parent_id).or_default().push(*child_id);
    }
    let counted: HashSet<i32> =
        budgets.iter().map(|b| b.category_id).chain(children.iter().map(|(child_id, _)| *child_id)).collect();

    // Plain and split transactions come through one converted contribution
    // seam, so every budget is compared with spending in the app currency.
    let mut spent_by_category: HashMap<i32, Decimal> = HashMap::new();
    for contribution in contributions {
        if counted.contains(&contribution.category_id) {
            let sum = spent_by_category.entry(contribution.category_id).or_insert_with(Decimal::zero);
            *sum = sum.add(&contribution.amount);
        }
    }
    let spent_on = |category_id: i32| spent_by_category.get(&category_id).cloned().unwrap_or_else(Decimal::zero);

    budgets
        .iter()
        .map(|budget| {
            let children_total = children_by_parent
                .get(&budget.category_id)
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .fold(Decimal::zero(), |sum, child_id| sum.add(&spent_on(*child_id)));
            let spent = spent_on(budget.category_id).add(&children_total);
            BudgetStatus {
                budget_id: budget.id,
                category_id: budget.category_id,
                category_name: budget.category_name.clone(),
                category_color: budget.category_color.clone(),
                category_icon: budget.category_icon.clone(),
                monthly_limit: budget.monthly_limit.clone(),
                remaining: budget.monthly_limit.sub(&spent),
                percent: percent_of(&spent, &budget.monthly_limit),
                is_over_budget: spent > budget.monthly_limit,
                spent,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::Source;
    use serde_json::json;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn budget(id: i32, category_id: i32, limit: &str) -> BudgetWithCategory {
        BudgetWithCategory {
            id,
            category_id,
            category_name: format!("c{category_id}"),
            category_color: "#000000".into(),
            category_icon: None,
            monthly_limit: d(limit),
        }
    }

    fn contribution(category_id: i32, amount: &str) -> CategoryContribution {
        CategoryContribution {
            transaction_id: 1,
            category_id,
            amount: d(amount),
            transaction_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            currency: "USD".into(),
        }
    }

    #[test]
    fn month_bounds_handle_leap_years_and_december() {
        let ymd = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert_eq!(month_bounds(2024, 2), Some((ymd(2024, 2, 1), ymd(2024, 2, 29))));
        assert_eq!(month_bounds(2026, 12), Some((ymd(2026, 12, 1), ymd(2026, 12, 31))));
    }

    #[test]
    fn parent_budget_rolls_children_up() {
        let budgets = vec![budget(1, 2, "100.00"), budget(2, 18, "10.00")];
        let children = vec![(18, 2), (19, 2)];
        let contributions =
            vec![contribution(2, "50.00"), contribution(18, "15.00"), contribution(19, "5.5"), contribution(7, "999")];
        let items = budget_status(&budgets, &children, &contributions);
        assert_eq!(items[0].spent.to_string(), "70.50");
        assert_eq!(items[0].remaining.to_string(), "29.50");
        assert_eq!(serde_json::to_string(&items[0].percent).unwrap(), "70.5");
        assert!(!items[0].is_over_budget);
        assert_eq!(items[1].spent.to_string(), "15.00");
        assert!(items[1].is_over_budget);
        assert_eq!(serde_json::to_string(&items[1].percent).unwrap(), "150.0");
    }

    #[test]
    fn unspent_budget_prints_python_zero() {
        let items = budget_status(&[budget(1, 2, "100.00")], &[], &[]);
        assert_eq!(items[0].spent.to_string(), "0");
        assert_eq!(items[0].remaining.to_string(), "100.00");
        assert_eq!(serde_json::to_string(&items[0].percent).unwrap(), "0.0");
    }

    #[test]
    fn status_query_bounds() {
        let errors =
            BudgetStatusQuery::from_source(&json!({"year": "1999", "month": "13"}), Source::Query).unwrap_err();
        assert_eq!(errors.0.len(), 2);
        let ok = BudgetStatusQuery::from_source(&json!({"month": "2"}), Source::Query).unwrap();
        assert_eq!(ok, BudgetStatusQuery { year: None, month: Some(2) });
    }
}
