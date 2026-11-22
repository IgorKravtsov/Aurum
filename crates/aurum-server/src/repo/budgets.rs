//! SQL for `/api/budgets` (port of the queries in the former Python backend's `app/services/budget_service.py`).

use aurum_domain::Decimal;
use sqlx::PgExecutor;

/// One stored budget row.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct BudgetRow {
    pub id: i32,
    pub category_id: i32,
    pub monthly_limit: Decimal,
}

/// `select(Budget).join(Category).order_by(Category.sort_order)` — the very
/// statement SQLAlchemy sends (budget columns only; the categories come in
/// a second, `selectinload`-style query), so Postgres breaks sort_order
/// ties exactly as it does under Python.
pub async fn list(db: impl PgExecutor<'_>) -> sqlx::Result<Vec<BudgetRow>> {
    sqlx::query_as(
        "SELECT budgets.id, budgets.category_id, budgets.monthly_limit, budgets.created_at, budgets.updated_at \
         FROM budgets JOIN categories ON categories.id = budgets.category_id ORDER BY categories.sort_order",
    )
    .fetch_all(db)
    .await
}

pub async fn get(db: impl PgExecutor<'_>, budget_id: i64) -> sqlx::Result<Option<BudgetRow>> {
    sqlx::query_as("SELECT id, category_id, monthly_limit FROM budgets WHERE id = $1")
        .bind(budget_id)
        .fetch_optional(db)
        .await
}

/// Whether the category already has its (unique) budget.
pub async fn exists_for_category(db: impl PgExecutor<'_>, category_id: i64) -> sqlx::Result<bool> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM budgets WHERE category_id = $1)")
        .bind(category_id)
        .fetch_one(db)
        .await
}

/// Returns the new id.
pub async fn insert(db: impl PgExecutor<'_>, category_id: i64, monthly_limit: &Decimal) -> sqlx::Result<i32> {
    sqlx::query_scalar("INSERT INTO budgets (category_id, monthly_limit) VALUES ($1, $2) RETURNING id")
        .bind(category_id)
        .bind(monthly_limit)
        .fetch_one(db)
        .await
}

/// `budget.monthly_limit = ...` (TimestampMixin: bumps `updated_at`).
pub async fn update_limit(db: impl PgExecutor<'_>, budget_id: i32, monthly_limit: &Decimal) -> sqlx::Result<()> {
    sqlx::query("UPDATE budgets SET monthly_limit = $2, updated_at = now() WHERE id = $1")
        .bind(budget_id)
        .bind(monthly_limit)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn delete(db: impl PgExecutor<'_>, budget_id: i32) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM budgets WHERE id = $1").bind(budget_id).execute(db).await?;
    Ok(())
}

/// `select(Category.id, Category.parent_id).where(Category.parent_id.in_(ids))`
/// — `(child_id, parent_id)` in Postgres' order.
pub async fn children_of(db: impl PgExecutor<'_>, parent_ids: &[i32]) -> sqlx::Result<Vec<(i32, i32)>> {
    sqlx::query_as("SELECT categories.id, categories.parent_id FROM categories WHERE categories.parent_id = ANY($1)")
        .bind(parent_ids)
        .fetch_all(db)
        .await
}
