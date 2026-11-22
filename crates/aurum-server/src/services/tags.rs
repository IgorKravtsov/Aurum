//! Business logic for `/api/tags` (inline in the former Python backend's `app/api/routes/tags.py`
//! under Python). Reference implementation — see PORTING.md.

use aurum_domain::tags::{TagCreate, TagRead};
use aurum_domain::text::py_strip;
use sqlx::PgPool;

use crate::error::{ApiError, ApiResult};
use crate::repo;

pub async fn list(pool: &PgPool) -> ApiResult<Vec<TagRead>> {
    Ok(repo::tags::list(pool).await?)
}

/// Case-insensitive dedup — the frontend's tag picker creates tags on the
/// fly as the user types, so "Georgia" and "georgia" typed on two different
/// transactions should end up as the same tag, not two. An existing match is
/// returned as-is (still with 201, like the Python route).
pub async fn create(pool: &PgPool, payload: TagCreate) -> ApiResult<TagRead> {
    let name = py_strip(&payload.name);
    if let Some(existing) = repo::tags::find_by_lower_name(pool, &name.to_lowercase()).await? {
        return Ok(existing);
    }
    Ok(repo::tags::insert(pool, name).await?)
}

pub async fn delete(pool: &PgPool, tag_id: i64) -> ApiResult<()> {
    if repo::tags::get(pool, tag_id).await?.is_none() {
        return Err(ApiError::not_found("Tag not found"));
    }
    repo::tags::delete(pool, tag_id).await?;
    Ok(())
}
