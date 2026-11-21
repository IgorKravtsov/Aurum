//! SQL for `/api/tags` (reference implementation — see PORTING.md).

use aurum_domain::tags::TagRead;
use sqlx::PgExecutor;

/// `select(Tag).order_by(Tag.name)` — ordering stays in SQL so Postgres'
/// collation decides it, exactly as under Python.
pub async fn list(db: impl PgExecutor<'_>) -> sqlx::Result<Vec<TagRead>> {
    sqlx::query_as("SELECT id, name FROM tags ORDER BY name").fetch_all(db).await
}

pub async fn get(db: impl PgExecutor<'_>, tag_id: i64) -> sqlx::Result<Option<TagRead>> {
    sqlx::query_as("SELECT id, name FROM tags WHERE id = $1").bind(tag_id).fetch_optional(db).await
}

/// `where(func.lower(Tag.name) == lowered)`; `lowered` is already lower-cased.
pub async fn find_by_lower_name(db: impl PgExecutor<'_>, lowered: &str) -> sqlx::Result<Option<TagRead>> {
    sqlx::query_as("SELECT id, name FROM tags WHERE lower(name) = $1").bind(lowered).fetch_optional(db).await
}

pub async fn insert(db: impl PgExecutor<'_>, name: &str) -> sqlx::Result<TagRead> {
    sqlx::query_as("INSERT INTO tags (name) VALUES ($1) RETURNING id, name").bind(name).fetch_one(db).await
}

/// Association rows in `transaction_tags` go with it (ON DELETE CASCADE).
pub async fn delete(db: impl PgExecutor<'_>, tag_id: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM tags WHERE id = $1").bind(tag_id).execute(db).await?;
    Ok(())
}
