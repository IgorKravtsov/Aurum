//! SQL for `/api/advice` (category totals come from `category_rollup`,
//! month summaries from `dashboard`).

use std::collections::{HashMap, HashSet};

use sqlx::PgExecutor;

/// `{row[0] for row in select(Budget.category_id)}`.
pub async fn budgeted_category_ids(db: impl PgExecutor<'_>) -> sqlx::Result<HashSet<i32>> {
    let ids: Vec<i32> = sqlx::query_scalar("SELECT category_id FROM budgets").fetch_all(db).await?;
    Ok(ids.into_iter().collect())
}

/// `session.get(Category, id).name` for each of `ids` that still exists.
pub async fn category_names(db: impl PgExecutor<'_>, ids: &[i32]) -> sqlx::Result<HashMap<i32, String>> {
    let rows: Vec<(i32, String)> =
        sqlx::query_as("SELECT id, name FROM categories WHERE id = ANY($1)").bind(ids).fetch_all(db).await?;
    Ok(rows.into_iter().collect())
}
