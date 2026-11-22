//! Business logic for `/api/budgets` — port of the former Python backend's `app/services/budget_service.py`
//! (the pure comparison lives in `aurum_domain::budgets`).
//!
//! Monthly category budgets: CRUD for the limits themselves, plus
//! [`get_budget_status`], which compares each limit against actual spend
//! for a given month — the data behind the Budget page's progress bars and
//! the budget_exceeded proactive alert (services/insights.rs).
//!
//! Response fidelity: Python's session never reloads after commit
//! (`expire_on_commit=False`), so create/update answer the *request's*
//! `monthly_limit` (e.g. `"250.5"`, not the stored `"250.50"`).

use aurum_domain::Decimal;
use aurum_domain::budgets::{
    BudgetCreate, BudgetRead, BudgetStatusResponse, BudgetUpdate, BudgetWithCategory, budget_status, month_bounds,
};
use aurum_domain::categories::CategoryRead;
use aurum_domain::enums::{CategoryKind, TransactionType};
use sqlx::PgPool;

use crate::error::{ApiError, ApiResult};
use crate::repo;
use crate::repo::budgets::BudgetRow;
use crate::services::category_rollup::category_contributions;
use crate::state::AppState;

fn read(budget: &BudgetRow, category: &CategoryRead, monthly_limit: Decimal) -> BudgetRead {
    BudgetRead {
        id: budget.id,
        category_id: budget.category_id,
        category_name: category.name.clone(),
        category_color: category.color.clone(),
        category_icon: category.icon.clone(),
        monthly_limit,
    }
}

/// `list_budgets`: every budget with its category, in category sort_order.
pub async fn list_with_categories(pool: &PgPool) -> ApiResult<Vec<BudgetWithCategory>> {
    let budgets = repo::budgets::list(pool).await?;
    if budgets.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<i32> = budgets.iter().map(|b| b.category_id).collect();
    let categories = repo::categories::get_many(pool, &ids).await?;
    // The FK cascades, so every budget has its category.
    Ok(budgets
        .into_iter()
        .filter_map(|b| {
            let category = categories.iter().find(|c| c.id == b.category_id)?;
            Some(BudgetWithCategory {
                id: b.id,
                category_id: b.category_id,
                category_name: category.name.clone(),
                category_color: category.color.clone(),
                category_icon: category.icon.clone(),
                monthly_limit: b.monthly_limit,
            })
        })
        .collect())
}

pub async fn list(pool: &PgPool) -> ApiResult<Vec<BudgetRead>> {
    Ok(list_with_categories(pool).await?.into_iter().map(BudgetRead::from).collect())
}

pub async fn create(pool: &PgPool, payload: BudgetCreate) -> ApiResult<BudgetRead> {
    let Some(category) = repo::categories::get(pool, payload.category_id).await? else {
        return Err(ApiError::bad_request("Category not found"));
    };
    if category.kind != CategoryKind::Expense {
        return Err(ApiError::bad_request("Budgets can only be set on expense categories"));
    }
    if repo::budgets::exists_for_category(pool, payload.category_id).await? {
        return Err(ApiError::bad_request(format!("'{}' already has a budget", category.name)));
    }
    let id = repo::budgets::insert(pool, payload.category_id, &payload.monthly_limit).await?;
    let row = BudgetRow { id, category_id: category.id, monthly_limit: payload.monthly_limit.clone() };
    Ok(read(&row, &category, payload.monthly_limit))
}

async fn get_or_404(pool: &PgPool, budget_id: i64) -> ApiResult<BudgetRow> {
    repo::budgets::get(pool, budget_id).await?.ok_or_else(|| ApiError::not_found("Budget not found"))
}

pub async fn update(pool: &PgPool, budget_id: i64, payload: BudgetUpdate) -> ApiResult<BudgetRead> {
    let budget = get_or_404(pool, budget_id).await?;
    repo::budgets::update_limit(pool, budget.id, &payload.monthly_limit).await?;
    let category = repo::categories::get(pool, i64::from(budget.category_id))
        .await?
        .ok_or_else(|| ApiError::internal(anyhow::anyhow!("budget {} has no category", budget.id)))?;
    Ok(read(&budget, &category, payload.monthly_limit))
}

pub async fn delete(pool: &PgPool, budget_id: i64) -> ApiResult<()> {
    let budget = get_or_404(pool, budget_id).await?;
    repo::budgets::delete(pool, budget.id).await?;
    Ok(())
}

/// Each budget's spend for `year`/`month` in the app currency, against its
/// limit. Public API for other services (insights' budget_exceeded alert).
pub async fn get_budget_status(state: &AppState, year: i32, month: u32) -> ApiResult<BudgetStatusResponse> {
    let (start, end) =
        month_bounds(year, month).ok_or_else(|| ApiError::internal(anyhow::anyhow!("invalid month {year}-{month}")))?;

    let budgets = list_with_categories(&state.pool).await?;
    if budgets.is_empty() {
        return Ok(BudgetStatusResponse { year, month, items: Vec::new() });
    }

    let budget_category_ids: Vec<i32> = budgets.iter().map(|b| b.category_id).collect();
    let children = repo::budgets::children_of(&state.pool, &budget_category_ids).await?;
    let contributions = category_contributions(state, TransactionType::Expense, Some(start), Some(end)).await?;
    Ok(BudgetStatusResponse { year, month, items: budget_status(&budgets, &children, &contributions) })
}
