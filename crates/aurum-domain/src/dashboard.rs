//! API contract types for `/api/dashboard` — port of the former Python backend's `app/schemas/dashboard.py`,
//! plus the pure aggregation of the former Python backend's `app/services/dashboard_service.py`
//! (the Overview page's four stat cards and the spending-by-category donut).
//!
//! Follows `crates/aurum-server/PORTING.md`: response DTOs derive
//! `Serialize` (field order = pydantic field order, inherited fields first),
//! request DTOs implement `validation::FromInput`.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::category_rollup::CategoryRollupItem;
use crate::decimal::Decimal;
use crate::enums::TransactionType;
use crate::net_worth::py_percent;
use crate::pyfmt::PyFloat;
use crate::validation::{FromInput, Val, ValidationErrors};

/// Categorical slots are capped at 8 (dataviz skill: a 9th series folds into
/// "Other", never a generated hue) — this is also the exact size of the
/// default category set.
pub const MAX_CHART_SLICES: usize = 8;
/// muted ink, reserved for the non-categorical rollup
pub const OTHER_SLICE_COLOR: &str = "#898781";

/// One subcategory's (or the parent's own direct, un-subcategorized) share
/// of a [`CategoryBreakdownItem`]'s total — see
/// `category_rollup::CategoryRollupChildItem`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CategoryBreakdownChildItem {
    pub category_id: i32,
    pub name: String,
    pub color: String,
    pub icon: Option<String>,
    pub amount: Decimal,
}

/// `CategoryBreakdownItem`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CategoryBreakdownItem {
    pub category_id: Option<i32>,
    pub name: String,
    pub color: String,
    pub icon: Option<String>,
    pub amount: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = f64))]
    pub percent: PyFloat,
    /// Populated only when this slice's spend came from more than one
    /// distinct category (subcategories, or a mix of the parent itself and
    /// its children) — e.g. a receipt split across "Groceries" subcategories.
    pub children: Vec<CategoryBreakdownChildItem>,
}

/// `DashboardSummary` — response of `GET /api/dashboard/summary`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DashboardSummary {
    pub year: i32,
    pub month: i32,
    pub real_income: Decimal,
    pub spent: Decimal,
    pub net: Decimal,
    pub transferred_out: Decimal,
    pub spending_by_category: Vec<CategoryBreakdownItem>,
}

/// Query of `GET /api/dashboard/summary`: `year` (2000..=2100) and `month`
/// (1..=12) default to today's (`default_factory`), which the server fills
/// in — `None` here means "absent".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DashboardQuery {
    pub year: Option<i64>,
    pub month: Option<i64>,
}

impl FromInput for DashboardQuery {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        // Query values are never null, so "optional" == "default_factory".
        let year = o.optional("year", |v| v.int()?.ge(2000)?.le(2100));
        let month = o.optional("month", |v| v.int()?.ge(1)?.le(12));
        o.finish()?;
        Ok(Self { year: year.value(), month: month.value() })
    }
}

/// `_month_bounds`: first and last day of a month (`None` for an invalid one).
pub fn month_bounds(year: i32, month: u32) -> Option<(NaiveDate, NaiveDate)> {
    let start = NaiveDate::from_ymd_opt(year, month, 1)?;
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)?
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)?
    };
    Some((start, next.pred_opt()?))
}

/// The month's converted transaction amounts by type, in row order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MonthTotals {
    pub income: Option<Decimal>,
    pub expense: Option<Decimal>,
    pub transfer: Option<Decimal>,
}

impl MonthTotals {
    /// `totals[row.type] = totals.get(row.type, Decimal("0")) + converted`.
    pub fn add(&mut self, type_: TransactionType, amount: &Decimal) {
        let slot = match type_ {
            TransactionType::Income => &mut self.income,
            TransactionType::Expense => &mut self.expense,
            TransactionType::Transfer => &mut self.transfer,
        };
        *slot = Some(slot.take().unwrap_or_else(Decimal::zero).add(amount));
    }
}

/// The tail of `get_dashboard_summary`: stat cards plus the top
/// [`MAX_CHART_SLICES`] categories, the rest folded into "Other".
pub fn build_summary(year: i32, month: i32, totals: MonthTotals, rollup: Vec<CategoryRollupItem>) -> DashboardSummary {
    let real_income = totals.income.unwrap_or_else(Decimal::zero);
    let spent = totals.expense.unwrap_or_else(Decimal::zero);
    let transferred_out = totals.transfer.unwrap_or_else(Decimal::zero);

    let mut rows = rollup;
    let rest_rows = if rows.len() > MAX_CHART_SLICES { rows.split_off(MAX_CHART_SLICES) } else { Vec::new() };

    let mut spending_by_category: Vec<CategoryBreakdownItem> = rows
        .into_iter()
        .map(|row| CategoryBreakdownItem {
            category_id: Some(row.category_id),
            name: row.name,
            color: row.color,
            icon: row.icon,
            percent: PyFloat(py_percent(&row.amount, &spent)),
            amount: row.amount,
            children: row
                .children
                .into_iter()
                .map(|child| CategoryBreakdownChildItem {
                    category_id: child.category_id,
                    name: child.name,
                    color: child.color,
                    icon: child.icon,
                    amount: child.amount,
                })
                .collect(),
        })
        .collect();

    if !rest_rows.is_empty() {
        let other_amount = rest_rows.iter().fold(Decimal::zero(), |acc, row| acc.add(&row.amount));
        spending_by_category.push(CategoryBreakdownItem {
            category_id: None,
            name: "Other".to_string(),
            color: OTHER_SLICE_COLOR.to_string(),
            icon: Some("more-horizontal".to_string()),
            percent: PyFloat(py_percent(&other_amount, &spent)),
            amount: other_amount,
            children: Vec::new(),
        });
    }

    DashboardSummary {
        year,
        month,
        net: real_income.sub(&spent),
        real_income,
        spent,
        transferred_out,
        spending_by_category,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(category_id: i32, amount: &str) -> CategoryRollupItem {
        CategoryRollupItem {
            category_id,
            name: format!("c{category_id}"),
            color: "#000000".into(),
            icon: None,
            sort_order: 0,
            amount: amount.parse().unwrap(),
            transaction_count: 1,
            children: Vec::new(),
        }
    }

    #[test]
    fn month_bounds_handle_december_and_leap_years() {
        let d = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert_eq!(month_bounds(2026, 12), Some((d(2026, 12, 1), d(2026, 12, 31))));
        assert_eq!(month_bounds(2028, 2), Some((d(2028, 2, 1), d(2028, 2, 29))));
    }

    #[test]
    fn folds_the_tail_into_other() {
        let mut totals = MonthTotals::default();
        totals.add(TransactionType::Expense, &"100.00".parse().unwrap());
        let rollup = (1..=10).map(|id| item(id, "10.00")).collect();
        let summary = build_summary(2026, 1, totals, rollup);
        assert_eq!(summary.spending_by_category.len(), 9);
        let other = &summary.spending_by_category[8];
        assert_eq!(other.category_id, None);
        assert_eq!(other.amount.to_string(), "20.00");
        assert_eq!(other.percent, PyFloat(20.0));
        assert_eq!(summary.real_income.to_string(), "0");
        assert_eq!(summary.net.to_string(), "-100.00");
    }
}
