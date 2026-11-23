//! Database wiring: pool, migrations, first-boot seed.

pub mod migrate;
pub mod seed;

use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use crate::config::Settings;

/// Opens the pool. Sized like SQLAlchemy's default (5 + 10 overflow).
pub async fn connect(settings: &Settings) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(15)
        .acquire_timeout(Duration::from_secs(30))
        // `pool_pre_ping=True` in session.py.
        .test_before_acquire(true)
        .connect_with(settings.connect_options())
        .await
}
