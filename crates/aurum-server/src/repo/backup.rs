//! SQL for `/api/backup`: a read of every table for the export and the
//! wholesale replace for the restore (port of the SQLAlchemy work in
//! the former Python backend's `app/services/backup_service.py`).
//!
//! Deliberately self-contained (its own SQL over the tables rather than other
//! groups' repos): the backup file format is frozen, the other groups' code
//! is not.
//!
//! Row order is part of the file: Python reads every table with a bare
//! `select(Model)` (no ORDER BY), so the export does the same and gets the
//! heap order; the restore inserts rows in file order, so an export right
//! after a restore reproduces the file. Integer columns are read as `int8`
//! because the backup DTOs carry Python's unbounded ints as `i64`; on insert
//! an out-of-range value fails the assignment cast, like asyncpg's overflow
//! did, and the restore is rolled back.

use std::collections::{HashMap, HashSet};

use aurum_domain::Decimal;
use aurum_domain::backup::{
    AccountBackup, AppSettingsBackup, AssetBackup, AssetValuationBackup, BackupPayload, BudgetBackup, CategoryBackup,
    CryptoHoldingBackup, CryptoPortfolioBackup, CryptoTransactionBackup, FALLBACK_PORTFOLIO_COLOR,
    FALLBACK_PORTFOLIO_NAME, FxRateBackup, GoalBackup, GoalContributionBackup, RecurringTransactionBackup, TagBackup,
    TransactionBackup, TransactionSplitBackup, categories_in_insert_order, max_id, needs_fallback_portfolio,
};
use chrono::{DateTime, NaiveDate, Utc};
use sqlx::PgConnection;

/// SQLAlchemy's `selectinload` batch size: tags are loaded for this many
/// transactions per query, like Python, so Postgres plans the same query.
const SELECTIN_CHUNK: usize = 500;

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// Every table, in heap order (see the module docs). `app_settings` is
/// `None` when the singleton row doesn't exist.
pub async fn read_all(conn: &mut PgConnection, payload: &mut BackupPayload) -> sqlx::Result<()> {
    payload.accounts = sqlx::query_as(
        "SELECT accounts.id::int8 AS id, accounts.name, accounts.type, accounts.currency, accounts.color, \
         accounts.is_archived FROM accounts",
    )
    .fetch_all(&mut *conn)
    .await?;
    payload.categories = sqlx::query_as(
        "SELECT categories.id::int8 AS id, categories.name, categories.kind, categories.icon, categories.color, \
         categories.sort_order::int8 AS sort_order, categories.is_default, categories.parent_id::int8 AS parent_id \
         FROM categories",
    )
    .fetch_all(&mut *conn)
    .await?;
    payload.tags = sqlx::query_as("SELECT tags.id::int8 AS id, tags.name FROM tags").fetch_all(&mut *conn).await?;
    payload.transactions = sqlx::query_as(
        "SELECT transactions.id::int8 AS id, transactions.account_id::int8 AS account_id, \
         transactions.category_id::int8 AS category_id, transactions.transfer_account_id::int8 AS transfer_account_id, \
         transactions.type, transactions.amount, transactions.destination_amount, transactions.description, \
         transactions.merchant, transactions.notes, transactions.date FROM transactions",
    )
    .fetch_all(&mut *conn)
    .await?;
    fill_tag_ids(conn, &mut payload.transactions).await?;
    payload.transaction_splits = sqlx::query_as(
        "SELECT transaction_splits.id::int8 AS id, transaction_splits.transaction_id::int8 AS transaction_id, \
         transaction_splits.category_id::int8 AS category_id, transaction_splits.amount, transaction_splits.note \
         FROM transaction_splits",
    )
    .fetch_all(&mut *conn)
    .await?;
    payload.assets = sqlx::query_as(
        "SELECT assets.id::int8 AS id, assets.name, assets.asset_class, assets.currency, assets.notes, \
         assets.capital_role, assets.monthly_cash_flow, assets.risk_level FROM assets",
    )
    .fetch_all(&mut *conn)
    .await?;
    payload.asset_valuations = sqlx::query_as(
        "SELECT asset_valuations.id::int8 AS id, asset_valuations.asset_id::int8 AS asset_id, \
         asset_valuations.value, asset_valuations.as_of_date FROM asset_valuations",
    )
    .fetch_all(&mut *conn)
    .await?;
    payload.crypto_portfolios = sqlx::query_as(
        "SELECT crypto_portfolios.id::int8 AS id, crypto_portfolios.name, crypto_portfolios.color, \
         crypto_portfolios.is_archived FROM crypto_portfolios",
    )
    .fetch_all(&mut *conn)
    .await?;
    payload.crypto_holdings = sqlx::query_as(
        "SELECT crypto_holdings.asset_id::int8 AS asset_id, crypto_holdings.portfolio_id::int8 AS portfolio_id, \
         crypto_holdings.coingecko_id, crypto_holdings.symbol, crypto_holdings.name, crypto_holdings.thumb_url, \
         crypto_holdings.last_price, crypto_holdings.price_change_1h, crypto_holdings.price_change_24h, \
         crypto_holdings.price_change_7d, crypto_holdings.price_change_30d, crypto_holdings.price_change_1y \
         FROM crypto_holdings",
    )
    .fetch_all(&mut *conn)
    .await?;
    payload.crypto_transactions = sqlx::query_as(
        "SELECT crypto_transactions.id::int8 AS id, crypto_transactions.asset_id::int8 AS asset_id, \
         crypto_transactions.type, crypto_transactions.quantity, crypto_transactions.price_per_unit, \
         crypto_transactions.date, crypto_transactions.note FROM crypto_transactions",
    )
    .fetch_all(&mut *conn)
    .await?;
    payload.budgets = sqlx::query_as(
        "SELECT budgets.id::int8 AS id, budgets.category_id::int8 AS category_id, budgets.monthly_limit FROM budgets",
    )
    .fetch_all(&mut *conn)
    .await?;
    payload.goals =
        sqlx::query_as("SELECT goals.id::int8 AS id, goals.name, goals.target_amount, goals.target_date FROM goals")
            .fetch_all(&mut *conn)
            .await?;
    payload.goal_contributions = sqlx::query_as(
        "SELECT goal_contributions.id::int8 AS id, goal_contributions.goal_id::int8 AS goal_id, \
         goal_contributions.amount, goal_contributions.date, goal_contributions.note FROM goal_contributions",
    )
    .fetch_all(&mut *conn)
    .await?;
    payload.recurring_transactions = sqlx::query_as(
        "SELECT recurring_transactions.id::int8 AS id, recurring_transactions.account_id::int8 AS account_id, \
         recurring_transactions.category_id::int8 AS category_id, \
         recurring_transactions.transfer_account_id::int8 AS transfer_account_id, recurring_transactions.type, \
         recurring_transactions.amount, recurring_transactions.destination_amount, recurring_transactions.description, \
         recurring_transactions.merchant, recurring_transactions.notes, recurring_transactions.frequency, \
         recurring_transactions.anchor_date, recurring_transactions.last_posted_date, recurring_transactions.is_active \
         FROM recurring_transactions",
    )
    .fetch_all(&mut *conn)
    .await?;
    payload.fx_rates = sqlx::query_as(
        "SELECT fx_rates.id::int8 AS id, fx_rates.currency, fx_rates.requested_date, fx_rates.effective_date, \
         fx_rates.rate_to_pln, fx_rates.provider, fx_rates.fetched_at FROM fx_rates",
    )
    .fetch_all(&mut *conn)
    .await?;
    if let Some(settings) = read_app_settings(conn).await? {
        payload.app_settings = settings;
    }
    Ok(())
}

/// `selectinload(Transaction.tags)`: SQLAlchemy's many-to-many query, kept
/// verbatim (no ORDER BY) and batched like Python, so each transaction's
/// `tag_ids` come out in the order Postgres returns them there.
async fn fill_tag_ids(conn: &mut PgConnection, transactions: &mut [TransactionBackup]) -> sqlx::Result<()> {
    let ids: Vec<i64> = transactions.iter().map(|t| t.id).collect();
    let mut tags: HashMap<i64, Vec<i64>> = HashMap::new();
    for chunk in ids.chunks(SELECTIN_CHUNK) {
        let rows: Vec<(i64, i64)> = sqlx::query_as(
            "SELECT transactions_1.id::int8, tags.id::int8 \
             FROM transactions AS transactions_1 \
             JOIN transaction_tags AS transaction_tags_1 ON transactions_1.id = transaction_tags_1.transaction_id \
             JOIN tags ON tags.id = transaction_tags_1.tag_id \
             WHERE transactions_1.id = ANY($1::int4[])",
        )
        .bind(chunk)
        .fetch_all(&mut *conn)
        .await?;
        for (transaction_id, tag_id) in rows {
            tags.entry(transaction_id).or_default().push(tag_id);
        }
    }
    for t in transactions {
        t.tag_ids = tags.remove(&t.id).unwrap_or_default();
    }
    Ok(())
}

/// `session.get(AppSettings, 1)`.
async fn read_app_settings(conn: &mut PgConnection) -> sqlx::Result<Option<AppSettingsBackup>> {
    sqlx::query_as(
        "SELECT currency, negative_cash_flow_threshold_months::int8 AS negative_cash_flow_threshold_months, \
         net_worth_decline_threshold_months::int8 AS net_worth_decline_threshold_months, \
         risky_allocation_threshold_percent::int8 AS risky_allocation_threshold_percent, \
         idle_cash_threshold_amount, idle_cash_threshold_days::int8 AS idle_cash_threshold_days \
         FROM app_settings WHERE id = 1",
    )
    .fetch_optional(conn)
    .await
}

// ---------------------------------------------------------------------------
// Restore
// ---------------------------------------------------------------------------

/// Deletes every backed-up table, children before parents. FX rates have no
/// relationships, but are replaced too because a restore promises a full
/// snapshot. `transaction_tags` goes with `transactions` (ON DELETE
/// CASCADE); `crypto_sync_state` is not part of a backup and stays.
pub async fn delete_all(conn: &mut PgConnection) -> sqlx::Result<()> {
    for sql in [
        "DELETE FROM fx_rates",
        "DELETE FROM asset_valuations",
        "DELETE FROM crypto_transactions",
        "DELETE FROM crypto_holdings",
        "DELETE FROM crypto_portfolios",
        "DELETE FROM budgets",
        "DELETE FROM goal_contributions",
        "DELETE FROM goals",
        "DELETE FROM recurring_transactions",
        "DELETE FROM transaction_splits",
        "DELETE FROM transactions",
        "DELETE FROM tags",
        "DELETE FROM assets",
        "DELETE FROM categories",
        "DELETE FROM accounts",
    ] {
        sqlx::query(sql).execute(&mut *conn).await?;
    }
    Ok(())
}

/// Inserts every row of the file, parents before children, each table in
/// file order. Columns with ORM-side defaults are all present in the file;
/// `created_at`/`updated_at` take their DB defaults and
/// `crypto_holdings.network` (not part of a backup) stays NULL.
pub async fn insert_all(conn: &mut PgConnection, p: &BackupPayload) -> sqlx::Result<()> {
    insert_accounts(conn, &p.accounts).await?;
    insert_categories(conn, &p.categories).await?;
    insert_assets(conn, &p.assets).await?;
    insert_tags(conn, &p.tags).await?;
    insert_transactions(conn, &p.transactions).await?;
    insert_transaction_tags(conn, &p.transactions).await?;
    insert_transaction_splits(conn, &p.transaction_splits).await?;
    insert_asset_valuations(conn, &p.asset_valuations).await?;
    insert_crypto_portfolios(conn, &p.crypto_portfolios).await?;
    // A pre-portfolios backup has no crypto_portfolios and every holding's
    // portfolio_id is None — those holdings get a freshly created fallback
    // portfolio, inserted after the file's own (it takes its id from the
    // sequence, exactly like the ORM object without an id did).
    let fallback = if needs_fallback_portfolio(p) { Some(insert_fallback_portfolio(conn).await?) } else { None };
    insert_crypto_holdings(conn, &p.crypto_holdings, fallback).await?;
    insert_crypto_transactions(conn, &p.crypto_transactions).await?;
    insert_budgets(conn, &p.budgets).await?;
    insert_goals(conn, &p.goals).await?;
    insert_goal_contributions(conn, &p.goal_contributions).await?;
    insert_recurring(conn, &p.recurring_transactions).await?;
    insert_fx_rates(conn, &p.fx_rates).await?;
    Ok(())
}

/// Bulk-inserting rows with explicit ids doesn't advance a table's id
/// sequence, so the next generated id would collide — bump each to the
/// file's max id. Tables with no rows in the file are left alone, as is
/// the fallback portfolio (it already took its id from the sequence).
pub async fn reset_sequences(conn: &mut PgConnection, p: &BackupPayload) -> sqlx::Result<()> {
    let tables = [
        ("accounts", max_id(&p.accounts, |r| r.id)),
        ("categories", max_id(&p.categories, |r| r.id)),
        ("tags", max_id(&p.tags, |r| r.id)),
        ("assets", max_id(&p.assets, |r| r.id)),
        ("transactions", max_id(&p.transactions, |r| r.id)),
        ("transaction_splits", max_id(&p.transaction_splits, |r| r.id)),
        ("asset_valuations", max_id(&p.asset_valuations, |r| r.id)),
        ("crypto_portfolios", max_id(&p.crypto_portfolios, |r| r.id)),
        ("crypto_transactions", max_id(&p.crypto_transactions, |r| r.id)),
        ("budgets", max_id(&p.budgets, |r| r.id)),
        ("goals", max_id(&p.goals, |r| r.id)),
        ("goal_contributions", max_id(&p.goal_contributions, |r| r.id)),
        ("recurring_transactions", max_id(&p.recurring_transactions, |r| r.id)),
        ("fx_rates", max_id(&p.fx_rates, |r| r.id)),
    ];
    for (table, max) in tables {
        if let Some(max) = max {
            sqlx::query("SELECT setval(pg_get_serial_sequence($1, 'id'), $2)")
                .bind(table)
                .bind(max)
                .execute(&mut *conn)
                .await?;
        }
    }
    Ok(())
}

/// Singleton row — updated in place, not deleted/recreated (no sequence to
/// reset, id is always 1); created when missing.
pub async fn upsert_app_settings(conn: &mut PgConnection, s: &AppSettingsBackup) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO app_settings (id, currency, negative_cash_flow_threshold_months, \
           net_worth_decline_threshold_months, risky_allocation_threshold_percent, idle_cash_threshold_amount, \
           idle_cash_threshold_days) \
         VALUES (1, $1, $2, $3, $4, $5, $6) \
         ON CONFLICT (id) DO UPDATE SET currency = EXCLUDED.currency, \
           negative_cash_flow_threshold_months = EXCLUDED.negative_cash_flow_threshold_months, \
           net_worth_decline_threshold_months = EXCLUDED.net_worth_decline_threshold_months, \
           risky_allocation_threshold_percent = EXCLUDED.risky_allocation_threshold_percent, \
           idle_cash_threshold_amount = EXCLUDED.idle_cash_threshold_amount, \
           idle_cash_threshold_days = EXCLUDED.idle_cash_threshold_days",
    )
    .bind(&s.currency)
    .bind(s.negative_cash_flow_threshold_months)
    .bind(s.net_worth_decline_threshold_months)
    .bind(s.risky_allocation_threshold_percent)
    .bind(&s.idle_cash_threshold_amount)
    .bind(s.idle_cash_threshold_days)
    .execute(conn)
    .await?;
    Ok(())
}

// One `INSERT … SELECT FROM UNNEST(column arrays)` per table: a single
// statement that keeps file order. Enum columns are bound as arrays of their
// stored DB *names* (`db_name()`), which is what the varchar column holds.

fn col<T, U>(rows: &[T], f: impl Fn(&T) -> U) -> Vec<U> {
    rows.iter().map(f).collect()
}

async fn insert_accounts(conn: &mut PgConnection, rows: &[AccountBackup]) -> sqlx::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO accounts (id, name, type, currency, color, is_archived) \
         SELECT * FROM UNNEST($1::int8[], $2::text[], $3::text[], $4::text[], $5::text[], $6::bool[])",
    )
    .bind(col(rows, |r| r.id))
    .bind(col(rows, |r| r.name.clone()))
    .bind(col(rows, |r| r.type_.db_name()))
    .bind(col(rows, |r| r.currency.clone()))
    .bind(col(rows, |r| r.color.clone()))
    .bind(col(rows, |r| r.is_archived))
    .execute(conn)
    .await?;
    Ok(())
}

/// Row by row: a subcategory listed ahead of its (non-top-level) parent
/// must fail the FK check like Python's one-statement-per-row insert did,
/// which a single multi-row statement (checked at its end) would not.
async fn insert_categories(conn: &mut PgConnection, rows: &[CategoryBackup]) -> sqlx::Result<()> {
    for r in categories_in_insert_order(rows) {
        sqlx::query(
            "INSERT INTO categories (id, name, kind, icon, color, sort_order, is_default, parent_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(r.id)
        .bind(&r.name)
        .bind(r.kind)
        .bind(&r.icon)
        .bind(&r.color)
        .bind(r.sort_order)
        .bind(r.is_default)
        .bind(r.parent_id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

async fn insert_assets(conn: &mut PgConnection, rows: &[AssetBackup]) -> sqlx::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO assets (id, name, asset_class, currency, notes, capital_role, monthly_cash_flow, risk_level) \
         SELECT * FROM UNNEST($1::int8[], $2::text[], $3::text[], $4::text[], $5::text[], $6::text[], \
           $7::numeric[], $8::text[])",
    )
    .bind(col(rows, |r| r.id))
    .bind(col(rows, |r| r.name.clone()))
    .bind(col(rows, |r| r.asset_class.db_name()))
    .bind(col(rows, |r| r.currency.clone()))
    .bind(col(rows, |r| r.notes.clone()))
    .bind(col(rows, |r| r.capital_role.db_name()))
    .bind(col(rows, |r| r.monthly_cash_flow.clone()))
    .bind(col(rows, |r| r.risk_level.db_name()))
    .execute(conn)
    .await?;
    Ok(())
}

async fn insert_tags(conn: &mut PgConnection, rows: &[TagBackup]) -> sqlx::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    sqlx::query("INSERT INTO tags (id, name) SELECT * FROM UNNEST($1::int8[], $2::text[])")
        .bind(col(rows, |r| r.id))
        .bind(col(rows, |r| r.name.clone()))
        .execute(conn)
        .await?;
    Ok(())
}

async fn insert_transactions(conn: &mut PgConnection, rows: &[TransactionBackup]) -> sqlx::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO transactions (id, account_id, category_id, transfer_account_id, type, amount, \
           destination_amount, description, merchant, notes, date) \
         SELECT * FROM UNNEST($1::int8[], $2::int8[], $3::int8[], $4::int8[], $5::text[], $6::numeric[], \
           $7::numeric[], $8::text[], $9::text[], $10::text[], $11::date[])",
    )
    .bind(col(rows, |r| r.id))
    .bind(col(rows, |r| r.account_id))
    .bind(col(rows, |r| r.category_id))
    .bind(col(rows, |r| r.transfer_account_id))
    .bind(col(rows, |r| r.type_.db_name()))
    .bind(col(rows, |r| r.amount.clone()))
    .bind(col(rows, |r| r.destination_amount.clone()))
    .bind(col(rows, |r| r.description.clone()))
    .bind(col(rows, |r| r.merchant.clone()))
    .bind(col(rows, |r| r.notes.clone()))
    .bind(col::<_, NaiveDate>(rows, |r| r.date))
    .execute(conn)
    .await?;
    Ok(())
}

/// Association rows, per transaction in `tag_ids` order. A tag listed twice
/// on one transaction is linked once — SQLAlchemy's collection collapsed
/// such duplicates instead of failing the restore.
async fn insert_transaction_tags(conn: &mut PgConnection, rows: &[TransactionBackup]) -> sqlx::Result<()> {
    let mut pairs: Vec<(i64, i64)> = Vec::new();
    for t in rows {
        let mut seen = HashSet::new();
        pairs.extend(t.tag_ids.iter().filter(|tag| seen.insert(**tag)).map(|tag| (t.id, *tag)));
    }
    if pairs.is_empty() {
        return Ok(());
    }
    sqlx::query("INSERT INTO transaction_tags (transaction_id, tag_id) SELECT * FROM UNNEST($1::int8[], $2::int8[])")
        .bind(col(&pairs, |p| p.0))
        .bind(col(&pairs, |p| p.1))
        .execute(conn)
        .await?;
    Ok(())
}

async fn insert_transaction_splits(conn: &mut PgConnection, rows: &[TransactionSplitBackup]) -> sqlx::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO transaction_splits (id, transaction_id, category_id, amount, note) \
         SELECT * FROM UNNEST($1::int8[], $2::int8[], $3::int8[], $4::numeric[], $5::text[])",
    )
    .bind(col(rows, |r| r.id))
    .bind(col(rows, |r| r.transaction_id))
    .bind(col(rows, |r| r.category_id))
    .bind(col(rows, |r| r.amount.clone()))
    .bind(col(rows, |r| r.note.clone()))
    .execute(conn)
    .await?;
    Ok(())
}

async fn insert_asset_valuations(conn: &mut PgConnection, rows: &[AssetValuationBackup]) -> sqlx::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO asset_valuations (id, asset_id, value, as_of_date) \
         SELECT * FROM UNNEST($1::int8[], $2::int8[], $3::numeric[], $4::date[])",
    )
    .bind(col(rows, |r| r.id))
    .bind(col(rows, |r| r.asset_id))
    .bind(col(rows, |r| r.value.clone()))
    .bind(col::<_, NaiveDate>(rows, |r| r.as_of_date))
    .execute(conn)
    .await?;
    Ok(())
}

async fn insert_crypto_portfolios(conn: &mut PgConnection, rows: &[CryptoPortfolioBackup]) -> sqlx::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO crypto_portfolios (id, name, color, is_archived) \
         SELECT * FROM UNNEST($1::int8[], $2::text[], $3::text[], $4::bool[])",
    )
    .bind(col(rows, |r| r.id))
    .bind(col(rows, |r| r.name.clone()))
    .bind(col(rows, |r| r.color.clone()))
    .bind(col(rows, |r| r.is_archived))
    .execute(conn)
    .await?;
    Ok(())
}

async fn insert_fallback_portfolio(conn: &mut PgConnection) -> sqlx::Result<i64> {
    sqlx::query_scalar(
        "INSERT INTO crypto_portfolios (name, color, is_archived) VALUES ($1, $2, false) RETURNING id::int8",
    )
    .bind(FALLBACK_PORTFOLIO_NAME)
    .bind(FALLBACK_PORTFOLIO_COLOR)
    .fetch_one(conn)
    .await
}

async fn insert_crypto_holdings(
    conn: &mut PgConnection,
    rows: &[CryptoHoldingBackup],
    fallback_portfolio_id: Option<i64>,
) -> sqlx::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO crypto_holdings (asset_id, portfolio_id, coingecko_id, symbol, name, thumb_url, last_price, \
           price_change_1h, price_change_24h, price_change_7d, price_change_30d, price_change_1y) \
         SELECT * FROM UNNEST($1::int8[], $2::int8[], $3::text[], $4::text[], $5::text[], $6::text[], \
           $7::numeric[], $8::numeric[], $9::numeric[], $10::numeric[], $11::numeric[], $12::numeric[])",
    )
    .bind(col(rows, |r| r.asset_id))
    .bind(col(rows, |r| r.portfolio_id.or(fallback_portfolio_id)))
    .bind(col(rows, |r| r.coingecko_id.clone()))
    .bind(col(rows, |r| r.symbol.clone()))
    .bind(col(rows, |r| r.name.clone()))
    .bind(col(rows, |r| r.thumb_url.clone()))
    .bind(col::<_, Option<Decimal>>(rows, |r| r.last_price.clone()))
    .bind(col(rows, |r| r.price_change_1h.clone()))
    .bind(col(rows, |r| r.price_change_24h.clone()))
    .bind(col(rows, |r| r.price_change_7d.clone()))
    .bind(col(rows, |r| r.price_change_30d.clone()))
    .bind(col(rows, |r| r.price_change_1y.clone()))
    .execute(conn)
    .await?;
    Ok(())
}

async fn insert_crypto_transactions(conn: &mut PgConnection, rows: &[CryptoTransactionBackup]) -> sqlx::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO crypto_transactions (id, asset_id, type, quantity, price_per_unit, date, note) \
         SELECT * FROM UNNEST($1::int8[], $2::int8[], $3::text[], $4::numeric[], $5::numeric[], $6::date[], \
           $7::text[])",
    )
    .bind(col(rows, |r| r.id))
    .bind(col(rows, |r| r.asset_id))
    .bind(col(rows, |r| r.type_.db_name()))
    .bind(col(rows, |r| r.quantity.clone()))
    .bind(col(rows, |r| r.price_per_unit.clone()))
    .bind(col::<_, NaiveDate>(rows, |r| r.date))
    .bind(col(rows, |r| r.note.clone()))
    .execute(conn)
    .await?;
    Ok(())
}

async fn insert_budgets(conn: &mut PgConnection, rows: &[BudgetBackup]) -> sqlx::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO budgets (id, category_id, monthly_limit) \
         SELECT * FROM UNNEST($1::int8[], $2::int8[], $3::numeric[])",
    )
    .bind(col(rows, |r| r.id))
    .bind(col(rows, |r| r.category_id))
    .bind(col(rows, |r| r.monthly_limit.clone()))
    .execute(conn)
    .await?;
    Ok(())
}

async fn insert_goals(conn: &mut PgConnection, rows: &[GoalBackup]) -> sqlx::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO goals (id, name, target_amount, target_date) \
         SELECT * FROM UNNEST($1::int8[], $2::text[], $3::numeric[], $4::date[])",
    )
    .bind(col(rows, |r| r.id))
    .bind(col(rows, |r| r.name.clone()))
    .bind(col(rows, |r| r.target_amount.clone()))
    .bind(col::<_, Option<NaiveDate>>(rows, |r| r.target_date))
    .execute(conn)
    .await?;
    Ok(())
}

async fn insert_goal_contributions(conn: &mut PgConnection, rows: &[GoalContributionBackup]) -> sqlx::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO goal_contributions (id, goal_id, amount, date, note) \
         SELECT * FROM UNNEST($1::int8[], $2::int8[], $3::numeric[], $4::date[], $5::text[])",
    )
    .bind(col(rows, |r| r.id))
    .bind(col(rows, |r| r.goal_id))
    .bind(col(rows, |r| r.amount.clone()))
    .bind(col::<_, NaiveDate>(rows, |r| r.date))
    .bind(col(rows, |r| r.note.clone()))
    .execute(conn)
    .await?;
    Ok(())
}

async fn insert_recurring(conn: &mut PgConnection, rows: &[RecurringTransactionBackup]) -> sqlx::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO recurring_transactions (id, account_id, category_id, transfer_account_id, type, amount, \
           destination_amount, description, merchant, notes, frequency, anchor_date, last_posted_date, is_active) \
         SELECT * FROM UNNEST($1::int8[], $2::int8[], $3::int8[], $4::int8[], $5::text[], $6::numeric[], \
           $7::numeric[], $8::text[], $9::text[], $10::text[], $11::text[], $12::date[], $13::date[], $14::bool[])",
    )
    .bind(col(rows, |r| r.id))
    .bind(col(rows, |r| r.account_id))
    .bind(col(rows, |r| r.category_id))
    .bind(col(rows, |r| r.transfer_account_id))
    .bind(col(rows, |r| r.type_.db_name()))
    .bind(col(rows, |r| r.amount.clone()))
    .bind(col(rows, |r| r.destination_amount.clone()))
    .bind(col(rows, |r| r.description.clone()))
    .bind(col(rows, |r| r.merchant.clone()))
    .bind(col(rows, |r| r.notes.clone()))
    .bind(col(rows, |r| r.frequency.db_name()))
    .bind(col::<_, NaiveDate>(rows, |r| r.anchor_date))
    .bind(col::<_, Option<NaiveDate>>(rows, |r| r.last_posted_date))
    .bind(col(rows, |r| r.is_active))
    .execute(conn)
    .await?;
    Ok(())
}

async fn insert_fx_rates(conn: &mut PgConnection, rows: &[FxRateBackup]) -> sqlx::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO fx_rates (id, currency, requested_date, effective_date, rate_to_pln, provider, fetched_at) \
         SELECT * FROM UNNEST($1::int8[], $2::text[], $3::date[], $4::date[], $5::numeric[], $6::text[], \
           $7::timestamptz[])",
    )
    .bind(col(rows, |r| r.id))
    .bind(col(rows, |r| r.currency.clone()))
    .bind(col::<_, NaiveDate>(rows, |r| r.requested_date))
    .bind(col::<_, NaiveDate>(rows, |r| r.effective_date))
    .bind(col(rows, |r| r.rate_to_pln.clone()))
    .bind(col(rows, |r| r.provider.clone()))
    .bind(col::<_, DateTime<Utc>>(rows, |r| r.fetched_at.0))
    .execute(conn)
    .await?;
    Ok(())
}
