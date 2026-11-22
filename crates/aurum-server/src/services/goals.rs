//! Business logic for `/api/goals` — port of the former Python backend's `app/services/goal_service.py`
//! (the progress maths live in `aurum_domain::goals`).
//!
//! Savings goals: CRUD for the goal itself, plus a running current_amount —
//! the sum of all logged contributions, computed on read rather than
//! stored, so it's never out of sync with the log.
//!
//! Response fidelity: `create` answers the request's values (Python never
//! reloads them); `update` and `add_contribution` re-read the stored row,
//! as Python's column select does.

use aurum_domain::goals::{GoalContributionCreate, GoalCreate, GoalRead, GoalUpdate, goal_read, new_goal_read};
use sqlx::PgPool;

use crate::error::{ApiError, ApiResult};
use crate::repo;
use crate::repo::goals::GoalWithTotal;

fn to_read(row: GoalWithTotal) -> GoalRead {
    goal_read(row.id, row.name, row.target_amount, row.target_date, row.current_amount)
}

async fn read_one(pool: &PgPool, goal_id: i32) -> ApiResult<GoalRead> {
    Ok(to_read(repo::goals::get_with_total(pool, goal_id).await?))
}

async fn get_or_404(pool: &PgPool, goal_id: i64) -> ApiResult<i32> {
    repo::goals::find(pool, goal_id).await?.ok_or_else(|| ApiError::not_found("Goal not found"))
}

pub async fn list(pool: &PgPool) -> ApiResult<Vec<GoalRead>> {
    Ok(repo::goals::list(pool).await?.into_iter().map(to_read).collect())
}

pub async fn create(pool: &PgPool, payload: GoalCreate) -> ApiResult<GoalRead> {
    let id = repo::goals::insert(pool, &payload).await?;
    Ok(new_goal_read(id, payload))
}

pub async fn update(pool: &PgPool, goal_id: i64, payload: GoalUpdate) -> ApiResult<GoalRead> {
    let id = get_or_404(pool, goal_id).await?;
    // An empty patch flushes nothing under SQLAlchemy (updated_at stays).
    if payload.name.is_set() || payload.target_amount.is_set() || payload.target_date.is_set() {
        repo::goals::apply_patch(pool, id, &payload).await?;
    }
    read_one(pool, id).await
}

pub async fn delete(pool: &PgPool, goal_id: i64) -> ApiResult<()> {
    let id = get_or_404(pool, goal_id).await?;
    repo::goals::delete(pool, id).await?;
    Ok(())
}

pub async fn add_contribution(pool: &PgPool, goal_id: i64, payload: GoalContributionCreate) -> ApiResult<GoalRead> {
    let id = get_or_404(pool, goal_id).await?;
    repo::goals::insert_contribution(pool, id, &payload.amount, payload.date, payload.note.as_deref()).await?;
    read_one(pool, id).await
}
