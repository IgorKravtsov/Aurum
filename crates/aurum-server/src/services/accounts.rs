//! Business logic for `/api/accounts` — port of the former Python backend's `app/services/account_service.py`.
//!
//! Account CRUD, plus each account's live balance — summed from its
//! transaction rows (see `aurum_domain::accounts::account_balances`) rather
//! than stored, the same "derive it, don't duplicate it" approach the net
//! worth service uses for Cash.

use std::collections::HashMap;

use aurum_domain::Decimal;
use aurum_domain::accounts::{AccountCreate, AccountUpdate, AccountWithBalance, account_balances, zero_balance};
use aurum_domain::validation::Patch;
use sqlx::PgPool;

use crate::error::{ApiError, ApiResult};
use crate::repo;

/// Every account's live balance by id (accounts without transactions are
/// absent — use `zero_balance`). Reusable by net worth / insights.
pub async fn balances(pool: &PgPool) -> ApiResult<HashMap<i32, Decimal>> {
    Ok(account_balances(repo::accounts::balance_movements(pool).await?))
}

pub async fn list(pool: &PgPool, include_archived: bool) -> ApiResult<Vec<AccountWithBalance>> {
    let accounts = repo::accounts::list(pool, include_archived).await?;
    let balances = balances(pool).await?;
    Ok(accounts
        .into_iter()
        .map(|account| {
            let balance = zero_balance(&balances, account.id);
            account.with_balance(balance)
        })
        .collect())
}

pub async fn create(pool: &PgPool, payload: AccountCreate) -> ApiResult<AccountWithBalance> {
    let account = repo::accounts::insert(pool, &payload).await?;
    // A brand-new account has no transactions yet — no need to query.
    Ok(account.with_balance(Decimal::zero()))
}

pub async fn update(pool: &PgPool, account_id: i64, payload: AccountUpdate) -> ApiResult<AccountWithBalance> {
    let Some(account) = repo::accounts::get(pool, account_id).await? else {
        return Err(ApiError::not_found("Account not found"));
    };
    if let Patch::Value(currency) = &payload.currency
        && *currency != account.currency
        && repo::accounts::has_currency_history(pool, account_id).await?
    {
        return Err(ApiError::conflict(
            "Account currency cannot be changed after transactions or recurring rules have been recorded",
        ));
    }
    let account = repo::accounts::apply_patch(pool, account_id, &payload)
        .await?
        .ok_or_else(|| ApiError::internal(anyhow::anyhow!("account {account_id} vanished during update")))?;
    let balances = balances(pool).await?;
    let balance = zero_balance(&balances, account.id);
    Ok(account.with_balance(balance))
}

pub async fn delete(pool: &PgPool, account_id: i64) -> ApiResult<()> {
    if repo::accounts::get(pool, account_id).await?.is_none() {
        return Err(ApiError::not_found("Account not found"));
    }
    repo::accounts::delete(pool, account_id).await?;
    Ok(())
}
