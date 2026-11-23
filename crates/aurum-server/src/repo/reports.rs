//! SQL for `/api/reports` (port of the queries in the former Python backend's `app/services/reports_service.py`;
//! the contribution queries themselves live in `repo::category_rollup`).

use sqlx::PgExecutor;

/// `select(Category.id).where(Category.parent_id == category_id)` — a
/// top-level category's subcategories, in Postgres' order.
pub async fn child_ids(db: impl PgExecutor<'_>, category_id: i32) -> sqlx::Result<Vec<i32>> {
    sqlx::query_scalar("SELECT categories.id FROM categories WHERE categories.parent_id = $1")
        .bind(category_id)
        .fetch_all(db)
        .await
}
