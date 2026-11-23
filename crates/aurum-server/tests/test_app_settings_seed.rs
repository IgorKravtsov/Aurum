//! Regression coverage for seed_default_app_settings() respecting
//! AURUM_DEFAULT_CURRENCY. An earlier version of the 9f3a2d7c5e11 migration
//! hardcoded the singleton app_settings row to currency='USD' at migration
//! time — since this function only creates the row when it's missing, that
//! pre-existing row silently defeated a non-USD AURUM_DEFAULT_CURRENCY on a
//! fresh install. The migration no longer inserts the row at all; this
//! function is now the sole place it gets created.
//!
//! Exercised black-box: the server's own seed (what `aurum-server seed`
//! runs) is run with AURUM_DEFAULT_CURRENCY set, and the row is read back
//! with raw SQL.

use crate::common::TestApp;

async fn settings_currency(app: &TestApp) -> Option<String> {
    sqlx::query_scalar("SELECT currency FROM app_settings WHERE id = 1").fetch_optional(app.pool()).await.unwrap()
}

#[tokio::test]
async fn test_seed_creates_the_row_with_the_configured_currency() {
    let app = TestApp::start().await;
    sqlx::query("DELETE FROM app_settings").execute(app.pool()).await.unwrap();

    // Was: monkeypatch default_currency to GBP + seed_default_app_settings(session).
    app.run_seed(|settings| settings.default_currency = "GBP".into()).await;

    let settings_row = settings_currency(&app).await;
    assert!(settings_row.is_some());
    assert_eq!(settings_row.as_deref(), Some("GBP"));
}

/// Self-healing only kicks in when the row is missing — an existing
/// row's currency (e.g. one the user changed via Settings after install)
/// must never be silently reset just because the app restarted.
#[tokio::test]
async fn test_seed_never_overwrites_an_existing_row() {
    let app = TestApp::start().await;
    sqlx::query("UPDATE app_settings SET currency = 'EUR' WHERE id = 1").execute(app.pool()).await.unwrap();

    // Was: monkeypatch default_currency to GBP + seed_default_app_settings(session).
    app.run_seed(|settings| settings.default_currency = "GBP".into()).await;

    assert_eq!(settings_currency(&app).await.as_deref(), Some("EUR"));
}
