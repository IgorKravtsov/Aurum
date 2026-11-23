//! `aurum-server` binary.
//!
//! ```text
//! aurum-server [serve]   migrate + seed, then serve HTTP (the default; what
//!                        the Python image did with `alembic upgrade head &&
//!                        uvicorn` plus the lifespan seeding)
//! aurum-server migrate   only bring the schema up to date
//! aurum-server seed      only insert the default categories/account/settings
//! ```
//!
//! Configuration comes from `AURUM_*` environment variables and `./.env`
//! (see `config.rs`).

use std::net::SocketAddr;

use anyhow::Context;
use aurum_server::config::Settings;
use aurum_server::db;
use aurum_server::state::AppState;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "aurum-server", version, about = "Aurum REST API server")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Clone, Copy)]
enum Command {
    /// Migrate, seed, then serve HTTP (default).
    Serve,
    /// Apply database migrations and exit.
    Migrate,
    /// Insert default data (idempotent) and exit.
    Seed,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx::postgres::notice=warn")),
        )
        .init();
    let cli = Cli::parse();
    let settings = Settings::from_env().context("invalid configuration")?;
    match cli.command.unwrap_or(Command::Serve) {
        Command::Migrate => {
            let pool = db::migrate::wait_for_database(&settings).await?;
            migrate(&pool).await
        }
        Command::Seed => {
            let pool = db::migrate::wait_for_database(&settings).await?;
            db::seed::run(&pool, &settings).await.context("seeding failed")
        }
        Command::Serve => serve(settings).await,
    }
}

async fn migrate(pool: &sqlx::PgPool) -> anyhow::Result<()> {
    let outcome = db::migrate::run(pool).await.context("migration failed")?;
    tracing::info!(?outcome, "database schema is up to date");
    Ok(())
}

async fn serve(settings: Settings) -> anyhow::Result<()> {
    let pool = db::migrate::wait_for_database(&settings).await?;
    migrate(&pool).await?;
    db::seed::run(&pool, &settings).await.context("seeding failed")?;

    let addr: SocketAddr = format!("{}:{}", settings.host, settings.port)
        .parse()
        .with_context(|| format!("invalid listen address {}:{}", settings.host, settings.port))?;
    let state = AppState::new(pool, settings)?;
    let app = aurum_server::build_app(state);
    let listener = tokio::net::TcpListener::bind(addr).await.with_context(|| format!("cannot listen on {addr}"))?;
    tracing::info!(%addr, "aurum-server listening");
    axum::serve(listener, app).with_graceful_shutdown(shutdown_signal()).await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
