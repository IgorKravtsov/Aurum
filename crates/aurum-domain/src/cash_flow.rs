//! API contract types for `/api/cash-flow` — port of `backend/app/schemas/cash_flow.py`,
//! plus the pure aggregation of `backend/app/services/cash_flow_service.py`.
//!
//! Month-by-month income vs. expense — the multi-month view neither the
//! Dashboard (locked to one month) nor Reports (one category at a time, or
//! a category ranking) answers on its own. Transfers between the user's own
//! accounts are excluded from both totals, same as the Dashboard breakdown.
//!
//! Follows `crates/aurum-server/PORTING.md`: response DTOs derive
//! `Serialize` (field order = pydantic field order, inherited fields first),
//! request DTOs implement `validation::FromInput`.

use std::collections::HashMap;

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::dashboard::MonthTotals;
use crate::decimal::Decimal;
use crate::enums::TransactionType;
use crate::validation::{FromInput, Val, ValidationErrors};

/// `CashFlowPoint`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CashFlowPoint {
    pub year: i32,
    pub month: u32,
    pub income: Decimal,
    pub expense: Decimal,
    pub net: Decimal,
}

/// `CashFlowResponse` — response of `GET /api/cash-flow`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CashFlowResponse {
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub points: Vec<CashFlowPoint>,
    pub total_income: Decimal,
    pub total_expense: Decimal,
    pub total_net: Decimal,
}

impl CashFlowResponse {
    /// No months to report (no income/expense in range, and no explicit bound).
    pub fn empty(start_date: Option<NaiveDate>, end_date: Option<NaiveDate>) -> Self {
        Self {
            start_date,
            end_date,
            points: Vec::new(),
            total_income: Decimal::zero(),
            total_expense: Decimal::zero(),
            total_net: Decimal::zero(),
        }
    }
}

/// Query of `GET /api/cash-flow`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CashFlowQuery {
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
}

impl FromInput for CashFlowQuery {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let start_date = o.optional("start_date", |v| v.date());
        let end_date = o.optional("end_date", |v| v.date());
        o.finish()?;
        Ok(Self { start_date: start_date.value(), end_date: end_date.value() })
    }
}

/// `_next_month`.
pub fn next_month(year: i32, month: u32) -> (i32, u32) {
    if month == 12 { (year + 1, 1) } else { (year, month + 1) }
}

/// One converted income/expense amount.
#[derive(Debug, Clone, PartialEq)]
pub struct CashFlowEntry {
    pub date: NaiveDate,
    pub type_: TransactionType,
    pub amount: Decimal,
}

/// The tail of `get_cash_flow`: one point per calendar month from
/// `start`'s month to `end`'s (months without activity read zero), summed
/// in row order like Python.
pub fn build_cash_flow(start: NaiveDate, end: NaiveDate, entries: &[CashFlowEntry]) -> CashFlowResponse {
    let mut by_month: HashMap<(i32, u32), MonthTotals> = HashMap::new();
    for entry in entries {
        by_month.entry((entry.date.year(), entry.date.month())).or_default().add(entry.type_, &entry.amount);
    }

    let mut points = Vec::new();
    let (mut year, mut month) = (start.year(), start.month());
    while (year, month) <= (end.year(), end.month()) {
        let totals = by_month.remove(&(year, month)).unwrap_or_default();
        let income = totals.income.unwrap_or_else(Decimal::zero);
        let expense = totals.expense.unwrap_or_else(Decimal::zero);
        points.push(CashFlowPoint { year, month, net: income.sub(&expense), income, expense });
        (year, month) = next_month(year, month);
    }

    let total_income = points.iter().fold(Decimal::zero(), |acc, p| acc.add(&p.income));
    let total_expense = points.iter().fold(Decimal::zero(), |acc, p| acc.add(&p.expense));
    CashFlowResponse {
        start_date: Some(start),
        end_date: Some(end),
        points,
        total_net: total_income.sub(&total_expense),
        total_income,
        total_expense,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn fills_every_month_in_range() {
        let entries = vec![
            CashFlowEntry { date: day(2025, 12, 5), type_: TransactionType::Income, amount: "10.00".parse().unwrap() },
            CashFlowEntry { date: day(2026, 2, 1), type_: TransactionType::Expense, amount: "4.50".parse().unwrap() },
        ];
        let response = build_cash_flow(day(2025, 12, 1), day(2026, 2, 28), &entries);
        assert_eq!(response.points.len(), 3);
        assert_eq!(response.points[1].income.to_string(), "0");
        assert_eq!(response.points[2].net.to_string(), "-4.50");
        assert_eq!(response.total_net.to_string(), "5.50");
        assert!(build_cash_flow(day(2026, 3, 1), day(2026, 2, 1), &entries).points.is_empty());
    }
}
