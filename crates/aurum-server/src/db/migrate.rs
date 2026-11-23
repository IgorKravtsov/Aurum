//! Schema migrations (ADR 0002).
//!
//! Runner choice: `sqlx::migrate!` (files in `crates/aurum-server/migrations`,
//! embedded at compile time, checksummed, tracked in `_sqlx_migrations`,
//! guarded by a Postgres advisory lock) — the standard tool, nothing custom
//! to maintain. The only custom part is adopting databases that Alembic
//! created, decided *before* sqlx runs:
//!
//! | database state                                   | action                          |
//! |--------------------------------------------------|---------------------------------|
//! | already has `_sqlx_migrations`                   | run pending migrations          |
//! | no tables at all                                 | run everything (incl. baseline) |
//! | `alembic_version` = [`ALEMBIC_HEAD`]             | mark baseline applied, then run the rest |
//! | `alembic_version` at another revision / empty    | refuse with an upgrade hint     |
//! | tables but no `alembic_version`                  | refuse (unknown origin)         |
//!
//! `alembic_version` itself is never modified, so the previous (Python)
//! release keeps working against the same database.

use std::time::Duration;

use sqlx::migrate::Migrator;
use sqlx::{Connection, PgConnection, PgPool};

/// The Alembic head the baseline corresponds to.
pub const ALEMBIC_HEAD: &str = "e4b9c2d7a105";
/// Version number of `migrations/0001_baseline.sql`.
pub const BASELINE_VERSION: i64 = 1;

/// Arbitrary constant for our own advisory lock around the adoption check
/// (sqlx takes its own lock while applying migrations).
const ADOPTION_LOCK_KEY: i64 = 0x4155_5255_4d00_0001; // "AURUM\0\0\x01"

pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

#[derive(Debug, thiserror::Error)]
pub enum MigrateError {
    #[error(
        "this database was created by the Python version of Aurum and is at Alembic revision {found}, \
         but this version needs revision {ALEMBIC_HEAD}. Upgrade it first: start the last Python release \
         of Aurum once against this database (it runs `alembic upgrade head` on startup), stop it, then \
         start this version again."
    )]
    AlembicBehind { found: String },
    #[error(
        "the database already contains tables but neither Alembic's nor Aurum's migration history \
         (`alembic_version` / `_sqlx_migrations`); refusing to touch it. Point AURUM_POSTGRES_DB at an \
         empty database or at an existing Aurum database."
    )]
    UnknownSchema,
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

/// What [`run`] did, for logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Empty database: baseline (and anything after it) executed.
    Fresh,
    /// Alembic database at head: baseline recorded without running.
    AdoptedAlembic,
    /// Already managed by sqlx: pending migrations (if any) applied.
    UpToDate,
}

/// Brings the schema to the latest version. Safe to run on every start.
pub async fn run(pool: &PgPool) -> Result<Outcome, MigrateError> {
    let mut conn = pool.acquire().await?;
    sqlx::query("SELECT pg_advisory_lock($1)").bind(ADOPTION_LOCK_KEY).execute(&mut *conn).await?;
    let result = run_locked(&mut conn).await;
    // Unlock even when migrating failed; the connection returns to the pool.
    let _ = sqlx::query("SELECT pg_advisory_unlock($1)").bind(ADOPTION_LOCK_KEY).execute(&mut *conn).await;
    result
}

async fn run_locked(conn: &mut PgConnection) -> Result<Outcome, MigrateError> {
    let outcome = if table_exists(conn, "_sqlx_migrations").await? {
        Outcome::UpToDate
    } else if table_exists(conn, "alembic_version").await? {
        let versions: Vec<String> =
            sqlx::query_scalar("SELECT version_num FROM public.alembic_version ORDER BY version_num")
                .fetch_all(&mut *conn)
                .await?;
        if versions != [ALEMBIC_HEAD] {
            let found = if versions.is_empty() { "<none>".to_string() } else { versions.join(", ") };
            return Err(MigrateError::AlembicBehind { found });
        }
        // Record the baseline as applied without executing it.
        MIGRATOR.run_direct(Some(BASELINE_VERSION), &mut *conn, true).await?;
        Outcome::AdoptedAlembic
    } else if public_table_count(conn).await? > 0 {
        return Err(MigrateError::UnknownSchema);
    } else {
        Outcome::Fresh
    };
    MIGRATOR.run_direct(None, &mut *conn, false).await?;
    Ok(outcome)
}

async fn table_exists(conn: &mut PgConnection, table: &str) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT to_regclass('public.' || quote_ident($1)) IS NOT NULL")
        .bind(table)
        .fetch_one(&mut *conn)
        .await
}

async fn public_table_count(conn: &mut PgConnection) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM information_schema.tables WHERE table_schema = 'public'")
        .fetch_one(&mut *conn)
        .await
}

/// Waits for Postgres to accept connections (the compose `db` service may
/// still be starting), then returns a pool.
pub async fn wait_for_database(settings: &crate::config::Settings) -> anyhow::Result<PgPool> {
    let mut attempt = 0u32;
    loop {
        match PgConnection::connect_with(&settings.connect_options()).await {
            Ok(conn) => {
                let _ = conn.close().await;
                return Ok(super::connect(settings).await?);
            }
            Err(err) if attempt < 30 => {
                attempt += 1;
                tracing::info!(attempt, error = %err, "database not reachable yet, retrying");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(err) => return Err(err.into()),
        }
    }
}
