//! API contract types for `/api/reports` — port of `backend/app/schemas/reports.py`,
//! plus the pure half of `backend/app/services/reports_service.py`.
//!
//! Long-range spending reports — the analysis the month-scoped Dashboard
//! breakdown can't answer on its own: "how much did I spend on X this month,
//! and how much in total over N years" ([`category_spending_report`]), and
//! "which categories cost the most over this whole period"
//! ([`category_ranking_report`]).
//!
//! Follows `crates/aurum-server/PORTING.md`: response DTOs derive
//! `Serialize` (field order = pydantic field order, inherited fields first),
//! request DTOs implement `validation::FromInput`.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::budgets::percent_of;
use crate::category_rollup::{CategoryContribution, CategoryRollupItem};
use crate::decimal::Decimal;
use crate::enums::{CategoryKind, TransactionType};
use crate::pyfmt::PyFloat;
use crate::validation::{FromInput, Val, ValidationErrors};

// ---------------------------------------------------------------------------
// Request DTOs
// ---------------------------------------------------------------------------

/// Query of `GET /api/reports/category-spending`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CategorySpendingQuery {
    pub category_id: i64,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
}

impl FromInput for CategorySpendingQuery {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let category_id = o.required("category_id", |v| v.int());
        let start_date = o.optional("start_date", |v| v.date());
        let end_date = o.optional("end_date", |v| v.date());
        o.finish()?;
        Ok(Self { category_id: category_id.value(), start_date: start_date.value(), end_date: end_date.value() })
    }
}

/// Query of `GET /api/reports/category-ranking`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CategoryRankingQuery {
    pub kind: CategoryKind,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
}

impl FromInput for CategoryRankingQuery {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let kind = o.with_default("kind", || CategoryKind::Expense, |v| v.enum_());
        let start_date = o.optional("start_date", |v| v.date());
        let end_date = o.optional("end_date", |v| v.date());
        o.finish()?;
        Ok(Self { kind: kind.value(), start_date: start_date.value(), end_date: end_date.value() })
    }
}

// ---------------------------------------------------------------------------
// Response DTOs
// ---------------------------------------------------------------------------

/// `CategoryRankingChildItem` — one subcategory's (or the parent's own
/// direct, un-subcategorized) share of a [`CategoryRankingItem`]'s total —
/// see `category_rollup::CategoryRollupChildItem`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CategoryRankingChildItem {
    pub category_id: i32,
    pub name: String,
    pub color: String,
    pub icon: Option<String>,
    pub amount: Decimal,
}

/// `CategorySpendingPoint`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CategorySpendingPoint {
    pub year: i32,
    pub month: u32,
    pub amount: Decimal,
}

/// `CategorySpendingReport`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CategorySpendingReport {
    pub category_id: i32,
    pub category_name: String,
    pub category_color: String,
    pub category_icon: Option<String>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub total_amount: Decimal,
    pub transaction_count: i64,
    pub average_per_month: Decimal,
    pub series: Vec<CategorySpendingPoint>,
}

/// `CategoryRankingItem`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CategoryRankingItem {
    pub category_id: i32,
    pub name: String,
    pub color: String,
    pub icon: Option<String>,
    pub amount: Decimal,
    pub percent: PyFloat,
    pub transaction_count: i64,
    // Populated only when this category's spend came from more than one
    // distinct category (subcategories, or a mix of itself and its
    // children) — e.g. a receipt split across "Groceries" subcategories.
    pub children: Vec<CategoryRankingChildItem>,
}

/// `CategoryRankingReport`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CategoryRankingReport {
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub total_amount: Decimal,
    pub items: Vec<CategoryRankingItem>,
}

// ---------------------------------------------------------------------------
// Pure logic
// ---------------------------------------------------------------------------

/// `_KIND_TO_TRANSACTION_TYPE`: which transactions a category kind reports on.
pub fn transaction_type_for(kind: CategoryKind) -> TransactionType {
    match kind {
        CategoryKind::Expense => TransactionType::Expense,
        CategoryKind::Income => TransactionType::Income,
    }
}

/// The category a spending report is about.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportCategory {
    pub id: i32,
    pub name: String,
    pub color: String,
    pub icon: Option<String>,
}

fn next_month(year: i32, month: u32) -> (i32, u32) {
    if month == 12 { (year + 1, 1) } else { (year, month + 1) }
}

/// The in-memory half of `get_category_spending_report`. `category_ids` is
/// the category plus — for a top-level one — its subcategories (a
/// subcategory picked directly shows just its own transactions);
/// `all_contributions` the period's converted contributions in seam order.
pub fn category_spending_report(
    category: ReportCategory,
    category_ids: &[i32],
    start_date: Option<NaiveDate>,
    end_date: Option<NaiveDate>,
    all_contributions: &[CategoryContribution],
) -> CategorySpendingReport {
    let contributions: Vec<&CategoryContribution> =
        all_contributions.iter().filter(|item| category_ids.contains(&item.category_id)).collect();

    let (Some(min_date), Some(max_date)) = (
        contributions.iter().map(|item| item.transaction_date).min(),
        contributions.iter().map(|item| item.transaction_date).max(),
    ) else {
        return CategorySpendingReport {
            category_id: category.id,
            category_name: category.name,
            category_color: category.color,
            category_icon: category.icon,
            start_date,
            end_date,
            total_amount: Decimal::zero(),
            transaction_count: 0,
            average_per_month: Decimal::zero(),
            series: Vec::new(),
        };
    };
    let effective_start = start_date.unwrap_or(min_date);
    let effective_end = end_date.unwrap_or(max_date);

    let mut by_month: HashMap<(i32, u32), Decimal> = HashMap::new();
    for item in &contributions {
        let key = (item.transaction_date.year(), item.transaction_date.month());
        let sum = by_month.entry(key).or_insert_with(Decimal::zero);
        *sum = sum.add(&item.amount);
    }

    let total_amount = contributions.iter().fold(Decimal::zero(), |sum, item| sum.add(&item.amount));
    // Distinct transactions, not rows — a transaction split across two of
    // these categories (e.g. parent + one of its children) must count once,
    // the same as a plain transaction filed under just one of them.
    let total_count = contributions.iter().map(|item| item.transaction_id).collect::<HashSet<_>>().len();

    let mut series = Vec::new();
    let (mut year, mut month) = (effective_start.year(), effective_start.month());
    while (year, month) <= (effective_end.year(), effective_end.month()) {
        let amount = by_month.get(&(year, month)).cloned().unwrap_or_else(Decimal::zero);
        series.push(CategorySpendingPoint { year, month, amount });
        (year, month) = next_month(year, month);
    }

    let average_per_month = match total_amount.div(&Decimal::from(series.len() as i64)) {
        Some(average) => average.quantize(-2),
        None => Decimal::zero(),
    };

    CategorySpendingReport {
        category_id: category.id,
        category_name: category.name,
        category_color: category.color,
        category_icon: category.icon,
        start_date: Some(effective_start),
        end_date: Some(effective_end),
        total_amount,
        transaction_count: total_count as i64,
        average_per_month,
        series,
    }
}

/// The in-memory half of `get_category_ranking_report`: all categories of
/// one kind, ranked by total spent/earned over an arbitrary period, with
/// each one's share of the whole.
pub fn category_ranking_report(
    rows: Vec<CategoryRollupItem>,
    start_date: Option<NaiveDate>,
    end_date: Option<NaiveDate>,
) -> CategoryRankingReport {
    let total_amount = rows.iter().fold(Decimal::zero(), |sum, row| sum.add(&row.amount));
    let items = rows
        .into_iter()
        .map(|row| CategoryRankingItem {
            percent: percent_of(&row.amount, &total_amount),
            category_id: row.category_id,
            name: row.name,
            color: row.color,
            icon: row.icon,
            amount: row.amount,
            transaction_count: row.transaction_count as i64,
            children: row
                .children
                .into_iter()
                .map(|child| CategoryRankingChildItem {
                    category_id: child.category_id,
                    name: child.name,
                    color: child.color,
                    icon: child.icon,
                    amount: child.amount,
                })
                .collect(),
        })
        .collect();
    CategoryRankingReport { start_date, end_date, total_amount, items }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn contribution(transaction_id: i32, category_id: i32, amount: &str, date: NaiveDate) -> CategoryContribution {
        CategoryContribution {
            transaction_id,
            category_id,
            amount: d(amount),
            transaction_date: date,
            currency: "USD".into(),
        }
    }

    fn category() -> ReportCategory {
        ReportCategory { id: 2, name: "Groceries".into(), color: "#000000".into(), icon: None }
    }

    #[test]
    fn spending_series_fills_gaps_and_averages() {
        let contributions = vec![
            contribution(1, 2, "10.00", ymd(2025, 11, 3)),
            contribution(2, 18, "5.00", ymd(2026, 1, 20)),
            contribution(2, 2, "1.00", ymd(2026, 1, 20)),
            contribution(3, 7, "99.00", ymd(2026, 1, 21)),
        ];
        let report = category_spending_report(category(), &[2, 18], None, None, &contributions);
        assert_eq!(report.total_amount.to_string(), "16.00");
        assert_eq!(report.transaction_count, 2);
        assert_eq!(report.series.len(), 3);
        assert_eq!(report.series[1].amount.to_string(), "0");
        assert_eq!(report.average_per_month.to_string(), "5.33");
        assert_eq!(report.start_date, Some(ymd(2025, 11, 3)));
    }

    #[test]
    fn empty_spending_report_keeps_query_dates() {
        let report = category_spending_report(category(), &[2], Some(ymd(2026, 1, 1)), None, &[]);
        assert_eq!(report.start_date, Some(ymd(2026, 1, 1)));
        assert_eq!(report.end_date, None);
        assert_eq!(report.total_amount.to_string(), "0");
        assert_eq!(report.average_per_month.to_string(), "0");
    }
}
