//! SQL for `/api/assets` (port of the queries in `backend/app/api/routes/assets.py`).

use aurum_domain::Decimal;
use aurum_domain::assets::{AssetCreate, AssetRow, AssetUpdate, AssetValuationRead};
use aurum_domain::validation::Patch;
use chrono::NaiveDate;
use sqlx::PgExecutor;

/// The asset columns plus its latest valuation (the last of the
/// `order_by=as_of_date` relationship). Correlated subqueries keep the
/// outer scan/sort exactly what `select(Asset).order_by(Asset.name)` ran,
/// so ties in the collation order come out the same.
macro_rules! asset_row_sql {
    ($tail:expr) => {
        concat!(
            "SELECT assets.id, assets.name, assets.asset_class, assets.currency, assets.notes, \
             assets.capital_role, assets.monthly_cash_flow, assets.risk_level, \
             (assets.created_at AT TIME ZONE 'UTC')::date AS created_date, \
             (SELECT v.value FROM asset_valuations v WHERE v.asset_id = assets.id \
              ORDER BY v.as_of_date DESC LIMIT 1) AS latest_value, \
             (SELECT v.as_of_date FROM asset_valuations v WHERE v.asset_id = assets.id \
              ORDER BY v.as_of_date DESC LIMIT 1) AS latest_date \
             FROM assets ",
            $tail
        )
    };
}

/// `select(Asset).options(selectinload(Asset.valuations)).order_by(Asset.name)`.
pub async fn list(db: impl PgExecutor<'_>) -> sqlx::Result<Vec<AssetRow>> {
    sqlx::query_as(asset_row_sql!("ORDER BY assets.name")).fetch_all(db).await
}

/// One asset with its latest valuation.
pub async fn get(db: impl PgExecutor<'_>, asset_id: i64) -> sqlx::Result<Option<AssetRow>> {
    sqlx::query_as(asset_row_sql!("WHERE assets.id = $1")).bind(asset_id).fetch_optional(db).await
}

/// `session.get(Asset, asset_id) is not None`.
pub async fn exists(db: impl PgExecutor<'_>, asset_id: i64) -> sqlx::Result<bool> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM assets WHERE id = $1)").bind(asset_id).fetch_one(db).await
}

/// Inserts the asset (every ORM-side default is in the payload already)
/// and returns its id.
pub async fn insert(db: impl PgExecutor<'_>, asset: &AssetCreate) -> sqlx::Result<i32> {
    sqlx::query_scalar(
        "INSERT INTO assets (name, asset_class, currency, notes, capital_role, monthly_cash_flow, risk_level) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
    )
    .bind(&asset.name)
    .bind(asset.asset_class)
    .bind(&asset.currency)
    .bind(&asset.notes)
    .bind(asset.capital_role)
    .bind(&asset.monthly_cash_flow)
    .bind(asset.risk_level)
    .fetch_one(db)
    .await
}

/// A plain insert of one valuation (the asset's first).
pub async fn insert_valuation(
    db: impl PgExecutor<'_>,
    asset_id: i32,
    value: &Decimal,
    as_of_date: NaiveDate,
) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO asset_valuations (asset_id, value, as_of_date) VALUES ($1, $2, $3)")
        .bind(asset_id)
        .bind(value)
        .bind(as_of_date)
        .execute(db)
        .await?;
    Ok(())
}

/// Records (or corrects) an asset's value as of a date: re-submitting the
/// same date updates that day's value (`ON CONFLICT (asset_id, as_of_date)`).
pub async fn upsert_valuation(
    db: impl PgExecutor<'_>,
    asset_id: i64,
    value: &Decimal,
    as_of_date: NaiveDate,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO asset_valuations (asset_id, value, as_of_date) VALUES ($1, $2, $3) \
         ON CONFLICT (asset_id, as_of_date) DO UPDATE SET value = $2",
    )
    .bind(asset_id)
    .bind(value)
    .bind(as_of_date)
    .execute(db)
    .await?;
    Ok(())
}

/// The asset's valuation history, oldest first.
pub async fn valuations(db: impl PgExecutor<'_>, asset_id: i64) -> sqlx::Result<Vec<AssetValuationRead>> {
    sqlx::query_as(
        "SELECT id, value, as_of_date FROM asset_valuations WHERE asset_id = $1 ORDER BY asset_valuations.as_of_date",
    )
    .bind(asset_id)
    .fetch_all(db)
    .await
}

/// `setattr(asset, field, value)` for every *set* field as one static
/// statement (see `repo::settings::apply_patch`). Like the ORM, `updated_at`
/// only moves when a column's value actually changed. An explicit `null`
/// for a NOT NULL column fails like the Python commit did (500).
pub async fn apply_patch(db: impl PgExecutor<'_>, asset_id: i64, patch: &AssetUpdate) -> sqlx::Result<()> {
    fn parts<T: Clone>(p: &Patch<T>) -> (bool, Option<T>) {
        (p.is_set(), p.as_value().cloned())
    }
    let (set_name, name) = parts(&patch.name);
    let (set_class, asset_class) = parts(&patch.asset_class);
    let (set_currency, currency) = parts(&patch.currency);
    let (set_notes, notes) = parts(&patch.notes);
    let (set_role, capital_role) = parts(&patch.capital_role);
    let (set_flow, monthly_cash_flow) = parts(&patch.monthly_cash_flow);
    let (set_risk, risk_level) = parts(&patch.risk_level);
    sqlx::query(
        "UPDATE assets SET \
           name = CASE WHEN $2 THEN $3 ELSE name END, \
           asset_class = CASE WHEN $4 THEN $5 ELSE asset_class END, \
           currency = CASE WHEN $6 THEN $7 ELSE currency END, \
           notes = CASE WHEN $8 THEN $9 ELSE notes END, \
           capital_role = CASE WHEN $10 THEN $11 ELSE capital_role END, \
           monthly_cash_flow = CASE WHEN $12 THEN $13 ELSE monthly_cash_flow END, \
           risk_level = CASE WHEN $14 THEN $15 ELSE risk_level END, \
           updated_at = CASE WHEN ($2 AND $3 IS DISTINCT FROM name) \
             OR ($4 AND $5 IS DISTINCT FROM asset_class) \
             OR ($6 AND $7 IS DISTINCT FROM currency) \
             OR ($8 AND $9 IS DISTINCT FROM notes) \
             OR ($10 AND $11 IS DISTINCT FROM capital_role) \
             OR ($12 AND $13 IS DISTINCT FROM monthly_cash_flow) \
             OR ($14 AND $15 IS DISTINCT FROM risk_level) \
             THEN now() ELSE updated_at END \
         WHERE id = $1",
    )
    .bind(asset_id)
    .bind(set_name)
    .bind(name)
    .bind(set_class)
    .bind(asset_class)
    .bind(set_currency)
    .bind(currency)
    .bind(set_notes)
    .bind(notes)
    .bind(set_role)
    .bind(capital_role)
    .bind(set_flow)
    .bind(monthly_cash_flow)
    .bind(set_risk)
    .bind(risk_level)
    .execute(db)
    .await?;
    Ok(())
}

/// Valuations go with it (ON DELETE CASCADE, like the ORM's
/// `cascade="all, delete-orphan"`), and so does a crypto holding built on it.
pub async fn delete(db: impl PgExecutor<'_>, asset_id: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM assets WHERE id = $1").bind(asset_id).execute(db).await?;
    Ok(())
}
