//! SQL for `/api/accounts` (port of the queries in the former Python backend's `app/services/account_service.py`).

use aurum_domain::accounts::{AccountCreate, AccountRead, AccountUpdate, BalanceMovement};
use aurum_domain::validation::Patch;
use sqlx::PgExecutor;

/// `select(Account).order_by(Account.name)`, optionally without archived
/// accounts. Ordering stays in SQL so Postgres' collation decides it.
pub async fn list(db: impl PgExecutor<'_>, include_archived: bool) -> sqlx::Result<Vec<AccountRead>> {
    let sql = if include_archived {
        "SELECT name, type, currency, color, id, is_archived FROM accounts ORDER BY name"
    } else {
        "SELECT name, type, currency, color, id, is_archived FROM accounts WHERE is_archived IS false ORDER BY name"
    };
    sqlx::query_as(sql).fetch_all(db).await
}

pub async fn get(db: impl PgExecutor<'_>, account_id: i64) -> sqlx::Result<Option<AccountRead>> {
    sqlx::query_as("SELECT name, type, currency, color, id, is_archived FROM accounts WHERE id = $1")
        .bind(account_id)
        .fetch_optional(db)
        .await
}

/// Several accounts by id (the eager load of `TransactionRead.account`).
pub async fn get_many(db: impl PgExecutor<'_>, ids: &[i32]) -> sqlx::Result<Vec<AccountRead>> {
    sqlx::query_as("SELECT name, type, currency, color, id, is_archived FROM accounts WHERE id = ANY($1)")
        .bind(ids)
        .fetch_all(db)
        .await
}

/// `(id, currency)` of the accounts among `ids` that exist.
pub async fn currencies(db: impl PgExecutor<'_>, ids: &[i64]) -> sqlx::Result<Vec<(i32, String)>> {
    sqlx::query_as("SELECT id, currency FROM accounts WHERE id = ANY($1)").bind(ids).fetch_all(db).await
}

/// `is_archived` has an ORM-side default only, so it is written explicitly.
pub async fn insert(db: impl PgExecutor<'_>, account: &AccountCreate) -> sqlx::Result<AccountRead> {
    sqlx::query_as(
        "INSERT INTO accounts (name, type, currency, color, is_archived) VALUES ($1, $2, $3, $4, false) \
         RETURNING name, type, currency, color, id, is_archived",
    )
    .bind(&account.name)
    .bind(account.type_)
    .bind(&account.currency)
    .bind(&account.color)
    .fetch_one(db)
    .await
}

/// Every transaction's balance-relevant columns (`_account_balances`).
pub async fn balance_movements(db: impl PgExecutor<'_>) -> sqlx::Result<Vec<BalanceMovement>> {
    sqlx::query_as("SELECT type, amount, destination_amount, account_id, transfer_account_id FROM transactions")
        .fetch_all(db)
        .await
}

/// Whether a transaction or recurring rule references the account on
/// either side — which locks its currency.
pub async fn has_currency_history(db: impl PgExecutor<'_>, account_id: i64) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM transactions WHERE account_id = $1 OR transfer_account_id = $1) \
         OR EXISTS (SELECT 1 FROM recurring_transactions WHERE account_id = $1 OR transfer_account_id = $1)",
    )
    .bind(account_id)
    .fetch_one(db)
    .await
}

/// `setattr(account, field, value)` for every *set* field as one static
/// statement (see `repo::settings::apply_patch`). An explicit `null` for a
/// NOT NULL column fails like the Python commit did (500).
pub async fn apply_patch(
    db: impl PgExecutor<'_>,
    account_id: i64,
    patch: &AccountUpdate,
) -> sqlx::Result<Option<AccountRead>> {
    fn parts<T: Clone>(p: &Patch<T>) -> (bool, Option<T>) {
        (p.is_set(), p.as_value().cloned())
    }
    let (set_name, name) = parts(&patch.name);
    let (set_type, type_) = parts(&patch.type_);
    let (set_currency, currency) = parts(&patch.currency);
    let (set_color, color) = parts(&patch.color);
    let (set_archived, archived) = parts(&patch.is_archived);
    sqlx::query_as(
        "UPDATE accounts SET \
           name = CASE WHEN $2 THEN $3 ELSE name END, \
           type = CASE WHEN $4 THEN $5 ELSE type END, \
           currency = CASE WHEN $6 THEN $7 ELSE currency END, \
           color = CASE WHEN $8 THEN $9 ELSE color END, \
           is_archived = CASE WHEN $10 THEN $11 ELSE is_archived END, \
           updated_at = now() \
         WHERE id = $1 \
         RETURNING name, type, currency, color, id, is_archived",
    )
    .bind(account_id)
    .bind(set_name)
    .bind(name)
    .bind(set_type)
    .bind(type_)
    .bind(set_currency)
    .bind(currency)
    .bind(set_color)
    .bind(color)
    .bind(set_archived)
    .bind(archived)
    .fetch_optional(db)
    .await
}

/// Its transactions go with it (ON DELETE CASCADE, like the ORM's
/// `cascade="all, delete-orphan"`); transfers *into* it keep their row with
/// `transfer_account_id = NULL`.
pub async fn delete(db: impl PgExecutor<'_>, account_id: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM accounts WHERE id = $1").bind(account_id).execute(db).await?;
    Ok(())
}
