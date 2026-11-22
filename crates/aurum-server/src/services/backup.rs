//! Business logic for `/api/backup` — port of the former Python backend's `app/services/backup_service.py`.
//!
//! Full-database JSON backup & restore. The export is every row of every
//! table as one portable JSON document a user can download from the browser
//! and re-upload later. Restore fully REPLACES existing data — it's a
//! snapshot restore, not a merge — so the whole operation runs in one DB
//! transaction: a corrupt or incompatible file is rejected (referential
//! checks run first, before any row is touched), and any failure during the
//! swap rolls the database back to exactly where it was, so a bad file never
//! leaves the app half-restored. The file format is shared with the Python
//! server in both directions (ADR 0002).

use aurum_domain::PyDateTime;
use aurum_domain::backup::{BackupPayload, check_version, validate_references};
use chrono::Utc;
use sqlx::PgPool;

use crate::error::{ApiError, ApiResult};
use crate::repo;

pub async fn export(pool: &PgPool) -> ApiResult<BackupPayload> {
    let mut payload = BackupPayload::new_export(PyDateTime(Utc::now()));
    let mut conn = pool.acquire().await?;
    repo::backup::read_all(&mut conn, &mut payload).await?;
    Ok(payload)
}

/// Version and references are checked before anything is touched; after
/// that, any database failure is reported as one opaque 400.
pub async fn restore(pool: &PgPool, payload: &BackupPayload) -> ApiResult<()> {
    check_version(payload).map_err(ApiError::bad_request)?;
    validate_references(payload).map_err(ApiError::bad_request)?;
    replace_all(pool, payload).await.map_err(|err| {
        // The error text is Postgres', naming tables, columns and
        // constraints — a free map of the schema for anyone probing the API.
        // It goes to the container log, where the operator can actually read
        // it; the client gets told only that nothing changed (the
        // transaction is rolled back when it is dropped).
        tracing::error!(error = ?err, "Backup restore failed");
        ApiError::bad_request("Restore failed, no changes were made")
    })
}

async fn replace_all(pool: &PgPool, payload: &BackupPayload) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    // Children before parents on delete, parents before children on insert.
    repo::backup::delete_all(&mut tx).await?;
    repo::backup::insert_all(&mut tx, payload).await?;
    repo::backup::reset_sequences(&mut tx, payload).await?;
    repo::backup::upsert_app_settings(&mut tx, &payload.app_settings).await?;
    tx.commit().await
}
