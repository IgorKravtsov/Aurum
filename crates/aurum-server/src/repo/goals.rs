//! SQL for `/api/goals` (port of the queries in `backend/app/services/goal_service.py`).

use aurum_domain::Decimal;
use aurum_domain::goals::{GoalCreate, GoalUpdate};
use aurum_domain::validation::Patch;
use chrono::NaiveDate;
use sqlx::PgExecutor;

/// A goal with its running total (`_SELECT_WITH_TOTAL`).
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct GoalWithTotal {
    pub id: i32,
    pub name: String,
    pub target_amount: Decimal,
    pub target_date: Option<NaiveDate>,
    pub current_amount: Decimal,
}

/// `coalesce(sum(...), 0)` prints `"0"` for a goal without contributions,
/// and keeps the column's scale (`"250.00"`) otherwise.
macro_rules! select_with_total {
    ($where:expr) => {
        concat!(
            "SELECT goals.id, goals.name, goals.target_amount, goals.target_date, \
             coalesce(sum(goal_contributions.amount), 0) AS current_amount \
             FROM goals LEFT OUTER JOIN goal_contributions ON goal_contributions.goal_id = goals.id",
            $where,
            " GROUP BY goals.id, goals.name, goals.target_amount, goals.target_date, goals.created_at \
             ORDER BY goals.created_at"
        )
    };
}

pub async fn list(db: impl PgExecutor<'_>) -> sqlx::Result<Vec<GoalWithTotal>> {
    sqlx::query_as(select_with_total!("")).fetch_all(db).await
}

pub async fn get_with_total(db: impl PgExecutor<'_>, goal_id: i32) -> sqlx::Result<GoalWithTotal> {
    sqlx::query_as(select_with_total!(" WHERE goals.id = $1")).bind(goal_id).fetch_one(db).await
}

/// The goal's id if it exists (`session.get(Goal, goal_id)`).
pub async fn find(db: impl PgExecutor<'_>, goal_id: i64) -> sqlx::Result<Option<i32>> {
    sqlx::query_scalar("SELECT id FROM goals WHERE id = $1").bind(goal_id).fetch_optional(db).await
}

/// Returns the new id.
pub async fn insert(db: impl PgExecutor<'_>, goal: &GoalCreate) -> sqlx::Result<i32> {
    sqlx::query_scalar("INSERT INTO goals (name, target_amount, target_date) VALUES ($1, $2, $3) RETURNING id")
        .bind(&goal.name)
        .bind(&goal.target_amount)
        .bind(goal.target_date)
        .fetch_one(db)
        .await
}

/// `setattr(goal, field, value)` for every *set* field as one static
/// statement (see `repo::settings::apply_patch`), bumping `updated_at`. An
/// explicit `null` for a NOT NULL column fails like the Python commit (500).
pub async fn apply_patch(db: impl PgExecutor<'_>, goal_id: i32, patch: &GoalUpdate) -> sqlx::Result<()> {
    fn parts<T: Clone>(p: &Patch<T>) -> (bool, Option<T>) {
        (p.is_set(), p.as_value().cloned())
    }
    let (set_name, name) = parts(&patch.name);
    let (set_target, target_amount) = parts(&patch.target_amount);
    let (set_date, target_date) = parts(&patch.target_date);
    sqlx::query(
        "UPDATE goals SET \
           name = CASE WHEN $2 THEN $3 ELSE name END, \
           target_amount = CASE WHEN $4 THEN $5 ELSE target_amount END, \
           target_date = CASE WHEN $6 THEN $7 ELSE target_date END, \
           updated_at = now() \
         WHERE id = $1",
    )
    .bind(goal_id)
    .bind(set_name)
    .bind(name)
    .bind(set_target)
    .bind(target_amount)
    .bind(set_date)
    .bind(target_date)
    .execute(db)
    .await?;
    Ok(())
}

/// Its contributions go with it (ON DELETE CASCADE).
pub async fn delete(db: impl PgExecutor<'_>, goal_id: i32) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM goals WHERE id = $1").bind(goal_id).execute(db).await?;
    Ok(())
}

pub async fn insert_contribution(
    db: impl PgExecutor<'_>,
    goal_id: i32,
    amount: &Decimal,
    date: NaiveDate,
    note: Option<&str>,
) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO goal_contributions (goal_id, amount, date, note) VALUES ($1, $2, $3, $4)")
        .bind(goal_id)
        .bind(amount)
        .bind(date)
        .bind(note)
        .execute(db)
        .await?;
    Ok(())
}
