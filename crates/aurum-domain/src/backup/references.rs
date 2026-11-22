//! Pure checks a restore runs before touching a single row — port of
//! `restore_backup`'s version check and `_validate_references` in
//! `backend/app/services/backup_service.py`. Each returns the HTTP 400
//! `detail` text verbatim, so clients can also vet a file before upload.

use std::collections::HashSet;

use super::{BACKUP_FORMAT_VERSION, BackupPayload, CategoryBackup};

/// An incompatible/future file is rejected before anything else.
pub fn check_version(payload: &BackupPayload) -> Result<(), String> {
    if payload.aurum_backup_version == BACKUP_FORMAT_VERSION {
        return Ok(());
    }
    Err(format!(
        "Unsupported backup version {} (this Aurum version supports {BACKUP_FORMAT_VERSION})",
        payload.aurum_backup_version
    ))
}

/// Every foreign key in the file must point at a row of the same file. The
/// first violation, in Python's check order, is the error.
pub fn validate_references(payload: &BackupPayload) -> Result<(), String> {
    let account_ids: HashSet<i64> = payload.accounts.iter().map(|r| r.id).collect();
    let category_ids: HashSet<i64> = payload.categories.iter().map(|r| r.id).collect();
    let asset_ids: HashSet<i64> = payload.assets.iter().map(|r| r.id).collect();
    let tag_ids: HashSet<i64> = payload.tags.iter().map(|r| r.id).collect();

    for c in &payload.categories {
        if let Some(parent_id) = c.parent_id.filter(|id| !category_ids.contains(id)) {
            return Err(format!("Category {} references unknown parent_id {parent_id}", c.id));
        }
    }

    for t in &payload.transactions {
        if !account_ids.contains(&t.account_id) {
            return Err(format!("Transaction {} references unknown account_id {}", t.id, t.account_id));
        }
        if let Some(id) = t.transfer_account_id.filter(|id| !account_ids.contains(id)) {
            return Err(format!("Transaction {} references unknown transfer_account_id {id}", t.id));
        }
        if let Some(id) = t.category_id.filter(|id| !category_ids.contains(id)) {
            return Err(format!("Transaction {} references unknown category_id {id}", t.id));
        }
        if let Some(id) = t.tag_ids.iter().find(|id| !tag_ids.contains(*id)) {
            return Err(format!("Transaction {} references unknown tag_id {id}", t.id));
        }
    }

    let transaction_ids: HashSet<i64> = payload.transactions.iter().map(|r| r.id).collect();
    for s in &payload.transaction_splits {
        if !transaction_ids.contains(&s.transaction_id) {
            return Err(format!("Transaction split {} references unknown transaction_id {}", s.id, s.transaction_id));
        }
        if let Some(id) = s.category_id.filter(|id| !category_ids.contains(id)) {
            return Err(format!("Transaction split {} references unknown category_id {id}", s.id));
        }
    }

    for v in &payload.asset_valuations {
        if !asset_ids.contains(&v.asset_id) {
            return Err(format!("Asset valuation {} references unknown asset_id {}", v.id, v.asset_id));
        }
    }

    let portfolio_ids: HashSet<i64> = payload.crypto_portfolios.iter().map(|p| p.id).collect();
    let holding_asset_ids: HashSet<i64> = payload.crypto_holdings.iter().map(|h| h.asset_id).collect();
    for h in &payload.crypto_holdings {
        if !asset_ids.contains(&h.asset_id) {
            return Err(format!("Crypto holding {} references unknown asset_id {}", h.asset_id, h.asset_id));
        }
        // portfolio_id is allowed to be None (a pre-portfolios backup) — that
        // case is resolved to an auto-created fallback portfolio at restore
        // time, not validated here.
        if let Some(id) = h.portfolio_id.filter(|id| !portfolio_ids.contains(id)) {
            return Err(format!("Crypto holding {} references unknown portfolio_id {id}", h.asset_id));
        }
    }

    for tx in &payload.crypto_transactions {
        if !holding_asset_ids.contains(&tx.asset_id) {
            return Err(format!("Crypto transaction {} references unknown asset_id {}", tx.id, tx.asset_id));
        }
    }

    for b in &payload.budgets {
        if !category_ids.contains(&b.category_id) {
            return Err(format!("Budget {} references unknown category_id {}", b.id, b.category_id));
        }
    }

    let goal_ids: HashSet<i64> = payload.goals.iter().map(|r| r.id).collect();
    for c in &payload.goal_contributions {
        if !goal_ids.contains(&c.goal_id) {
            return Err(format!("Goal contribution {} references unknown goal_id {}", c.id, c.goal_id));
        }
    }

    for r in &payload.recurring_transactions {
        if !account_ids.contains(&r.account_id) {
            return Err(format!("Recurring transaction {} references unknown account_id {}", r.id, r.account_id));
        }
        if let Some(id) = r.transfer_account_id.filter(|id| !account_ids.contains(id)) {
            return Err(format!("Recurring transaction {} references unknown transfer_account_id {id}", r.id));
        }
        if let Some(id) = r.category_id.filter(|id| !category_ids.contains(id)) {
            return Err(format!("Recurring transaction {} references unknown category_id {id}", r.id));
        }
    }

    Ok(())
}

/// Categories are self-referential (parent_id points at another row of the
/// same table): top-level ones go first so a subcategory's FK is never
/// inserted ahead of the row it points to. Stable, like Python's `sorted`.
pub fn categories_in_insert_order(categories: &[CategoryBackup]) -> Vec<&CategoryBackup> {
    let mut ordered: Vec<&CategoryBackup> = categories.iter().collect();
    ordered.sort_by_key(|c| c.parent_id.is_some());
    ordered
}

/// A pre-portfolios backup has holdings without a portfolio_id; they get a
/// freshly created fallback portfolio on restore.
pub fn needs_fallback_portfolio(payload: &BackupPayload) -> bool {
    payload.crypto_holdings.iter().any(|h| h.portfolio_id.is_none())
}

/// `max(row.id for row in rows)` — what a table's id sequence is moved to
/// after rows were inserted with explicit ids; `None` for no rows (the
/// sequence is then left alone).
pub fn max_id<T>(rows: &[T], id: impl Fn(&T) -> i64) -> Option<i64> {
    rows.iter().map(id).max()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::{AccountBackup, CryptoHoldingBackup, TransactionBackup};
    use crate::enums::{AccountType, CategoryKind, TransactionType};
    use crate::pyfmt::PyDateTime;

    fn payload() -> BackupPayload {
        let mut p = BackupPayload::new_export(PyDateTime("2026-01-01T00:00:00Z".parse().unwrap()));
        p.accounts.push(AccountBackup {
            id: 1,
            name: "A".into(),
            type_: AccountType::Checking,
            currency: "USD".into(),
            color: None,
            is_archived: false,
        });
        p
    }

    fn category(id: i64, parent_id: Option<i64>) -> CategoryBackup {
        CategoryBackup {
            id,
            name: format!("c{id}"),
            kind: CategoryKind::Expense,
            icon: None,
            color: "#000000".into(),
            sort_order: 0,
            is_default: false,
            parent_id,
        }
    }

    fn transaction(id: i64, account_id: i64, tag_ids: Vec<i64>) -> TransactionBackup {
        TransactionBackup {
            id,
            account_id,
            category_id: None,
            transfer_account_id: None,
            type_: TransactionType::Expense,
            amount: "1.00".parse().unwrap(),
            destination_amount: None,
            description: "x".into(),
            merchant: None,
            notes: None,
            date: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            tag_ids,
        }
    }

    #[test]
    fn version_mismatch_is_reported() {
        let mut p = payload();
        assert_eq!(check_version(&p), Ok(()));
        p.aurum_backup_version = 2;
        assert_eq!(check_version(&p), Err("Unsupported backup version 2 (this Aurum version supports 1)".into()));
    }

    #[test]
    fn first_dangling_reference_wins() {
        let mut p = payload();
        assert_eq!(validate_references(&p), Ok(()));
        p.transactions.push(transaction(7, 1, vec![99]));
        assert_eq!(validate_references(&p), Err("Transaction 7 references unknown tag_id 99".into()));
        p.categories.push(category(3, Some(42)));
        assert_eq!(validate_references(&p), Err("Category 3 references unknown parent_id 42".into()));
    }

    #[test]
    fn holdings_without_portfolio_are_allowed() {
        let mut p = payload();
        p.crypto_holdings.push(CryptoHoldingBackup {
            asset_id: 5,
            portfolio_id: None,
            coingecko_id: "bitcoin".into(),
            symbol: "BTC".into(),
            name: "Bitcoin".into(),
            thumb_url: None,
            last_price: None,
            price_change_1h: None,
            price_change_24h: None,
            price_change_7d: None,
            price_change_30d: None,
            price_change_1y: None,
        });
        assert_eq!(validate_references(&p), Err("Crypto holding 5 references unknown asset_id 5".into()));
        assert!(needs_fallback_portfolio(&p));
    }

    #[test]
    fn top_level_categories_first_stable() {
        let cats = vec![category(1, Some(2)), category(2, None), category(3, Some(2)), category(4, None)];
        let ids: Vec<i64> = categories_in_insert_order(&cats).iter().map(|c| c.id).collect();
        assert_eq!(ids, vec![2, 4, 1, 3]);
        assert_eq!(max_id(&cats, |c| c.id), Some(4));
        assert_eq!(max_id::<CategoryBackup>(&[], |c| c.id), None);
    }
}
