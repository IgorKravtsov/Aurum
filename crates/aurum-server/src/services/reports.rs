//! Business logic for `/api/reports` — port of the former Python backend's `app/services/reports_service.py`
//! (the report maths live in `aurum_domain::reports`).
//!
//! Plain and split transactions share the currency-aware contribution seam
//! (`services::category_rollup`), so these reports use the same historical
//! FX rates as the Dashboard, budgets and advice.

use aurum_domain::enums::CategoryKind;
use aurum_domain::reports::{
    CategoryRankingReport, CategorySpendingReport, ReportCategory, category_ranking_report, category_spending_report,
    transaction_type_for,
};
use chrono::NaiveDate;

use crate::error::{ApiError, ApiResult};
use crate::repo;
use crate::services::category_rollup::{category_contributions, rollup_spending_by_top_level_category};
use crate::state::AppState;

/// One category's spend (or income) over a period, month by month.
pub async fn get_category_spending_report(
    state: &AppState,
    category_id: i64,
    start_date: Option<NaiveDate>,
    end_date: Option<NaiveDate>,
) -> ApiResult<CategorySpendingReport> {
    let Some(category) = repo::categories::get(&state.pool, category_id).await? else {
        return Err(ApiError::not_found("Category not found"));
    };

    // A top-level category's own report folds in its subcategories' spending
    // too (same rollup as the Dashboard breakdown); a subcategory picked
    // directly shows just its own transactions — there's nothing beneath it.
    let mut category_ids = vec![category.id];
    if category.parent_id.is_none() {
        category_ids.extend(repo::reports::child_ids(&state.pool, category.id).await?);
    }

    let contributions =
        category_contributions(state, transaction_type_for(category.kind), start_date, end_date).await?;
    let report_category =
        ReportCategory { id: category.id, name: category.name, color: category.color, icon: category.icon };
    Ok(category_spending_report(report_category, &category_ids, start_date, end_date, &contributions))
}

/// All categories of one kind, ranked by total spent/earned over an
/// arbitrary period — "which category costs the most" across the whole
/// range, unlike the month-scoped Dashboard breakdown or the
/// single-category detail above.
///
/// A transaction's type already restricts it to categories of the matching
/// kind (enforced at write time), so filtering by transaction type is
/// enough — no separate kind filter needed.
pub async fn get_category_ranking_report(
    state: &AppState,
    kind: CategoryKind,
    start_date: Option<NaiveDate>,
    end_date: Option<NaiveDate>,
) -> ApiResult<CategoryRankingReport> {
    let rows = rollup_spending_by_top_level_category(state, transaction_type_for(kind), start_date, end_date).await?;
    Ok(category_ranking_report(rows, start_date, end_date))
}
