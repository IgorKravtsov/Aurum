//! Business logic for `/api/categories` (inline in
//! the former Python backend's `app/api/routes/categories.py` under Python).

use aurum_domain::categories::{CategoryCreate, CategoryRead, CategoryUpdate, parent_rule_violation};
use aurum_domain::enums::CategoryKind;
use aurum_domain::validation::Patch;
use sqlx::PgPool;

use crate::error::{ApiError, ApiResult};
use crate::repo;

/// Subcategories are one level deep only: a parent must itself be
/// top-level, and must share the child's kind (`_validate_parent`).
async fn validate_parent(pool: &PgPool, parent_id: i64, kind: CategoryKind, category_id: Option<i64>) -> ApiResult<()> {
    let parent = if Some(parent_id) == category_id { None } else { repo::categories::get(pool, parent_id).await? };
    match parent_rule_violation(parent_id, kind, category_id, parent.as_ref()) {
        Some(detail) => Err(ApiError::bad_request(detail)),
        None => Ok(()),
    }
}

pub async fn list(pool: &PgPool, kind: Option<CategoryKind>) -> ApiResult<Vec<CategoryRead>> {
    Ok(repo::categories::list(pool, kind).await?)
}

pub async fn create(pool: &PgPool, payload: CategoryCreate) -> ApiResult<CategoryRead> {
    if let Some(parent_id) = payload.parent_id {
        validate_parent(pool, parent_id, payload.kind, None).await?;
    }
    Ok(repo::categories::insert(pool, &payload).await?)
}

pub async fn update(pool: &PgPool, category_id: i64, payload: CategoryUpdate) -> ApiResult<CategoryRead> {
    let Some(category) = repo::categories::get(pool, category_id).await? else {
        return Err(ApiError::not_found("Category not found"));
    };
    if let Patch::Value(parent_id) = payload.parent_id {
        validate_parent(pool, parent_id, category.kind, Some(category_id)).await?;
        if repo::categories::has_children(pool, category_id).await? {
            return Err(ApiError::bad_request("A category with subcategories cannot become a subcategory itself"));
        }
    }
    repo::categories::apply_patch(pool, category_id, &payload)
        .await?
        .ok_or_else(|| ApiError::internal(anyhow::anyhow!("category {category_id} vanished during update")))
}

pub async fn delete(pool: &PgPool, category_id: i64) -> ApiResult<()> {
    let Some(category) = repo::categories::get(pool, category_id).await? else {
        return Err(ApiError::not_found("Category not found"));
    };
    // A default (seeded) category can be removed once it's unused — but
    // never while transactions still point at it, or a whole year's worth of
    // history would silently lose its category (the FK is ON DELETE SET
    // NULL, so nothing would error, it would just vanish from every report).
    // A custom category has no such guard: the user created it and can
    // freely delete it.
    if category.is_default && repo::categories::is_used(pool, category_id).await? {
        return Err(ApiError::bad_request("Default categories can only be deleted once they have no transactions"));
    }
    repo::categories::delete(pool, category_id).await?;
    Ok(())
}
