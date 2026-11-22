//! Seeds the default category set on first boot — port of
//! the former Python backend's `app/db/seed.py`, insert for insert, so a fresh database gets the
//! same ids, names, icons, colors and sort orders as under Python.
//!
//! The expense categories are assigned hues from the dataviz skill's validated
//! 8-slot categorical palette, in the palette's fixed slot order (never
//! reordered/cycled) so the dashboard donut chart is colorblind-safe out of the
//! box. See CLAUDE.md-adjacent design notes in UPDATES.md for the source.

use aurum_domain::Decimal;
use aurum_domain::enums::{AccountType, CategoryKind};
use sqlx::PgPool;

use crate::config::Settings;

// (name, icon, color) — order doubles as sort_order / palette slot index.
pub const DEFAULT_EXPENSE_CATEGORIES: [(&str, &str, &str); 8] = [
    ("Housing & Utilities", "home", "#2a78d6"),     // slot 1 blue
    ("Groceries", "shopping-basket", "#1baf7a"),    // slot 3 aqua
    ("Dining Out", "utensils", "#eb6834"),          // slot 2 orange
    ("Transportation", "car", "#4a3aa7"),           // slot 7 violet
    ("Health & Fitness", "heart-pulse", "#e34948"), // slot 8 red
    ("Shopping", "shopping-bag", "#eda100"),        // slot 4 yellow
    ("Entertainment", "clapperboard", "#e87ba4"),   // slot 5 magenta
    ("Subscriptions", "repeat", "#008300"),         // slot 6 green
];

pub const DEFAULT_INCOME_CATEGORIES: [(&str, &str, &str); 9] = [
    ("Salary", "banknote", "#2a78d6"),
    ("Freelance", "briefcase", "#1baf7a"),
    ("Investments", "trending-up", "#4a3aa7"),
    ("Gifts", "gift", "#e87ba4"),
    ("Business Income", "building-2", "#eb6834"),
    ("Rental Income", "key", "#eda100"),
    ("Benefits", "hand-coins", "#008300"),
    ("Item Sales", "tag", "#e34948"),
    ("Other Income", "plus-circle", "#898781"),
];

/// Python model-level defaults of `AppSettings` (SQLAlchemy `default=`,
/// i.e. applied by the ORM, not by the database — so every INSERT has to
/// spell them out).
pub const DEFAULT_NEGATIVE_CASH_FLOW_THRESHOLD_MONTHS: i32 = 2;
pub const DEFAULT_NET_WORTH_DECLINE_THRESHOLD_MONTHS: i32 = 2;
pub const DEFAULT_RISKY_ALLOCATION_THRESHOLD_PERCENT: i32 = 20;
pub const DEFAULT_IDLE_CASH_THRESHOLD_AMOUNT: &str = "1000";
pub const DEFAULT_IDLE_CASH_THRESHOLD_DAYS: i32 = 60;

/// Runs all three seeds in the order main.py's lifespan does.
pub async fn run(pool: &PgPool, settings: &Settings) -> Result<(), sqlx::Error> {
    seed_default_categories(pool).await?;
    seed_default_account(pool, settings).await?;
    seed_default_app_settings(pool, settings).await?;
    Ok(())
}

/// Only when the categories table is completely empty.
pub async fn seed_default_categories(pool: &PgPool) -> Result<(), sqlx::Error> {
    let existing: Option<i32> = sqlx::query_scalar("SELECT id FROM categories LIMIT 1").fetch_optional(pool).await?;
    if existing.is_some() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    let expense = DEFAULT_EXPENSE_CATEGORIES.iter().map(|c| (CategoryKind::Expense, c));
    let income = DEFAULT_INCOME_CATEGORIES.iter().map(|c| (CategoryKind::Income, c));
    for (order, (kind, (name, icon, color))) in expense.chain(income).enumerate() {
        sqlx::query(
            "INSERT INTO categories (name, kind, icon, color, sort_order, is_default, parent_id) \
             VALUES ($1, $2, $3, $4, $5, true, NULL)",
        )
        .bind(*name)
        .bind(kind)
        .bind(*icon)
        .bind(*color)
        .bind(order as i32)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// Creates one starter account so the Transactions form always has a
/// destination to post to, even before the (future) accounts management UI
/// ships.
pub async fn seed_default_account(pool: &PgPool, settings: &Settings) -> Result<(), sqlx::Error> {
    let existing: Option<i32> = sqlx::query_scalar("SELECT id FROM accounts LIMIT 1").fetch_optional(pool).await?;
    if existing.is_some() {
        return Ok(());
    }
    sqlx::query("INSERT INTO accounts (name, type, currency, color, is_archived) VALUES ($1, $2, $3, $4, false)")
        .bind("Main Account")
        .bind(AccountType::Checking)
        .bind(&settings.default_currency)
        .bind("#2a78d6")
        .execute(pool)
        .await?;
    Ok(())
}

/// Ensures the singleton app_settings row (id=1) exists, seeded with
/// AURUM_DEFAULT_CURRENCY. The sole place that row gets created on boot — the
/// 9f3a2d7c5e11 migration only creates the table, not the row itself.
/// Runs on every start, so it's also self-healing if the row is ever missing
/// (e.g. a DB restored from a pre-currency-setting backup).
pub async fn seed_default_app_settings(pool: &PgPool, settings: &Settings) -> Result<(), sqlx::Error> {
    insert_app_settings_if_missing(pool, &settings.default_currency).await
}

/// Inserts the id=1 settings row with the model defaults if it's missing.
/// Shared with the settings service's get-or-create (which, like Python's
/// `AppSettings(id=1)`, uses the model default currency "USD" rather than
/// AURUM_DEFAULT_CURRENCY).
pub async fn insert_app_settings_if_missing(pool: &PgPool, currency: &str) -> Result<(), sqlx::Error> {
    let amount: Decimal = DEFAULT_IDLE_CASH_THRESHOLD_AMOUNT.parse().expect("valid default");
    sqlx::query(
        "INSERT INTO app_settings (id, currency, negative_cash_flow_threshold_months, \
         net_worth_decline_threshold_months, risky_allocation_threshold_percent, \
         idle_cash_threshold_amount, idle_cash_threshold_days) \
         VALUES (1, $1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING",
    )
    .bind(currency)
    .bind(DEFAULT_NEGATIVE_CASH_FLOW_THRESHOLD_MONTHS)
    .bind(DEFAULT_NET_WORTH_DECLINE_THRESHOLD_MONTHS)
    .bind(DEFAULT_RISKY_ALLOCATION_THRESHOLD_PERCENT)
    .bind(amount)
    .bind(DEFAULT_IDLE_CASH_THRESHOLD_DAYS)
    .execute(pool)
    .await?;
    Ok(())
}
