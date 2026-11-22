//! SQL for `/api/categories` (port of the queries in the former Python backend's `app/api/routes/categories.py`).

use aurum_domain::categories::{CategoryCreate, CategoryRead, CategoryUpdate};
use aurum_domain::category_rollup::RollupCategory;
use aurum_domain::enums::CategoryKind;
use aurum_domain::validation::Patch;
use sqlx::PgExecutor;

/// `select(Category).order_by(Category.sort_order)`, optionally filtered by
/// kind. Ties keep Postgres' order, exactly as under Python.
pub async fn list(db: impl PgExecutor<'_>, kind: Option<CategoryKind>) -> sqlx::Result<Vec<CategoryRead>> {
    match kind {
        None => sqlx::query_as(
            "SELECT name, kind, icon, color, sort_order, parent_id, id, is_default FROM categories ORDER BY sort_order",
        )
        .fetch_all(db)
        .await,
        Some(kind) => {
            sqlx::query_as(
                "SELECT name, kind, icon, color, sort_order, parent_id, id, is_default FROM categories \
                 WHERE kind = $1 ORDER BY sort_order",
            )
            .bind(kind)
            .fetch_all(db)
            .await
        }
    }
}

pub async fn get(db: impl PgExecutor<'_>, category_id: i64) -> sqlx::Result<Option<CategoryRead>> {
    sqlx::query_as(
        "SELECT name, kind, icon, color, sort_order, parent_id, id, is_default FROM categories WHERE id = $1",
    )
    .bind(category_id)
    .fetch_optional(db)
    .await
}

/// Several categories by id (eager loads of `TransactionRead.category` and
/// `TransactionSplitRead.category`).
pub async fn get_many(db: impl PgExecutor<'_>, ids: &[i32]) -> sqlx::Result<Vec<CategoryRead>> {
    sqlx::query_as(
        "SELECT name, kind, icon, color, sort_order, parent_id, id, is_default FROM categories WHERE id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(db)
    .await
}

/// Every category, for the rollup (`select(Category)`).
pub async fn all_for_rollup(db: impl PgExecutor<'_>) -> sqlx::Result<Vec<RollupCategory>> {
    sqlx::query_as("SELECT id, name, color, icon, sort_order, parent_id FROM categories").fetch_all(db).await
}

/// User-created categories are never defaults (`is_default=False`).
pub async fn insert(db: impl PgExecutor<'_>, category: &CategoryCreate) -> sqlx::Result<CategoryRead> {
    sqlx::query_as(
        "INSERT INTO categories (name, kind, icon, color, sort_order, parent_id, is_default) \
         VALUES ($1, $2, $3, $4, $5, $6, false) \
         RETURNING name, kind, icon, color, sort_order, parent_id, id, is_default",
    )
    .bind(&category.name)
    .bind(category.kind)
    .bind(&category.icon)
    .bind(&category.color)
    .bind(category.sort_order)
    .bind(category.parent_id)
    .fetch_one(db)
    .await
}

pub async fn has_children(db: impl PgExecutor<'_>, category_id: i64) -> sqlx::Result<bool> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM categories WHERE parent_id = $1)")
        .bind(category_id)
        .fetch_one(db)
        .await
}

/// Whether a transaction or a split line still points at the category.
pub async fn is_used(db: impl PgExecutor<'_>, category_id: i64) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM transactions WHERE category_id = $1) \
         OR EXISTS (SELECT 1 FROM transaction_splits WHERE category_id = $1)",
    )
    .bind(category_id)
    .fetch_one(db)
    .await
}

/// `setattr(category, field, value)` for every *set* field (see
/// `repo::settings::apply_patch`). An explicit `null` for a NOT NULL column
/// fails like the Python commit did (500).
pub async fn apply_patch(
    db: impl PgExecutor<'_>,
    category_id: i64,
    patch: &CategoryUpdate,
) -> sqlx::Result<Option<CategoryRead>> {
    fn parts<T: Clone>(p: &Patch<T>) -> (bool, Option<T>) {
        (p.is_set(), p.as_value().cloned())
    }
    let (set_name, name) = parts(&patch.name);
    let (set_icon, icon) = parts(&patch.icon);
    let (set_color, color) = parts(&patch.color);
    let (set_sort, sort_order) = parts(&patch.sort_order);
    let (set_parent, parent_id) = parts(&patch.parent_id);
    sqlx::query_as(
        "UPDATE categories SET \
           name = CASE WHEN $2 THEN $3 ELSE name END, \
           icon = CASE WHEN $4 THEN $5 ELSE icon END, \
           color = CASE WHEN $6 THEN $7 ELSE color END, \
           sort_order = CASE WHEN $8 THEN $9::int8 ELSE sort_order END, \
           parent_id = CASE WHEN $10 THEN $11::int8 ELSE parent_id END \
         WHERE id = $1 \
         RETURNING name, kind, icon, color, sort_order, parent_id, id, is_default",
    )
    .bind(category_id)
    .bind(set_name)
    .bind(name)
    .bind(set_icon)
    .bind(icon)
    .bind(set_color)
    .bind(color)
    .bind(set_sort)
    .bind(sort_order)
    .bind(set_parent)
    .bind(parent_id)
    .fetch_optional(db)
    .await
}

/// Transactions, split lines and subcategories pointing at it are set to
/// NULL by the foreign keys (ON DELETE SET NULL).
pub async fn delete(db: impl PgExecutor<'_>, category_id: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM categories WHERE id = $1").bind(category_id).execute(db).await?;
    Ok(())
}
