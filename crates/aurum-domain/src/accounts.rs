//! API contract types for `/api/accounts` — port of `backend/app/schemas/account.py`,
//! plus the pure balance calculation of `backend/app/services/account_service.py`.
//!
//! Follows `crates/aurum-server/PORTING.md`: response DTOs derive
//! `Serialize` (field order = pydantic field order, inherited fields first),
//! request DTOs implement `validation::FromInput`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::decimal::Decimal;
use crate::enums::{AccountType, TransactionType};
use crate::validation::{FromInput, Patch, Val, ValidationErrors};

/// `currency: str = Field(min_length=3, max_length=3, pattern=...)`.
pub const CURRENCY_PATTERN: &str = r"^[A-Za-z]{3}$";
/// Hex color used for account-scoped UI accents (e.g. transaction list avatars).
pub const COLOR_PATTERN: &str = r"^#[0-9a-fA-F]{6}$";

/// `AccountCreate` (= `AccountBase`) — body of `POST /api/accounts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AccountCreate {
    pub name: String,
    #[serde(rename = "type")]
    pub type_: AccountType,
    pub currency: String,
    pub color: Option<String>,
}

impl FromInput for AccountCreate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let name = o.required("name", |v| v.str()?.min_len(1)?.max_len(100));
        let type_ = o.with_default("type", || AccountType::Checking, |v| v.enum_());
        // `_normalise_currency` field validator: stored upper-case. Like any
        // pydantic field validator it doesn't run on the default.
        let currency = o.with_default(
            "currency",
            || "USD".to_string(),
            |v| Ok(v.str()?.min_len(3)?.max_len(3)?.pattern(CURRENCY_PATTERN)?.map(|s| s.to_uppercase())),
        );
        let color = o.optional("color", |v| v.str()?.pattern(COLOR_PATTERN));
        o.finish()?;
        Ok(Self { name: name.value(), type_: type_.value(), currency: currency.value(), color: color.value() })
    }
}

/// `AccountUpdate` — partial update (`exclude_unset`). An explicit `null`
/// for a NOT NULL column is written as NULL and fails at commit (500), as
/// under Python.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AccountUpdate {
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub name: Patch<String>,
    #[serde(rename = "type", skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<AccountType>))]
    pub type_: Patch<AccountType>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub currency: Patch<String>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub color: Patch<String>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<bool>))]
    pub is_archived: Patch<bool>,
}

impl FromInput for AccountUpdate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let name = o.patch("name", |v| v.str()?.min_len(1)?.max_len(100));
        let type_ = o.patch("type", |v| v.enum_());
        let currency = o.patch("currency", |v| {
            Ok(v.str()?.min_len(3)?.max_len(3)?.pattern(CURRENCY_PATTERN)?.map(|s| s.to_uppercase()))
        });
        let color = o.patch("color", |v| v.str()?.pattern(COLOR_PATTERN));
        let is_archived = o.patch("is_archived", |v| v.bool());
        o.finish()?;
        Ok(Self {
            name: name.value(),
            type_: type_.value(),
            currency: currency.value(),
            color: color.value(),
            is_archived: is_archived.value(),
        })
    }
}

/// `AccountRead` — used wherever an account is embedded in another response
/// (e.g. `TransactionRead.account`) — deliberately balance-less so adding a
/// field here can never break an unrelated endpoint's serialization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct AccountRead {
    pub name: String,
    #[serde(rename = "type")]
    #[cfg_attr(feature = "sqlx", sqlx(rename = "type"))]
    pub type_: AccountType,
    pub currency: String,
    pub color: Option<String>,
    pub id: i32,
    pub is_archived: bool,
}

impl AccountRead {
    /// pydantic re-runs `_normalise_currency` when it validates a row into
    /// `AccountRead`, so the serialised currency is always upper-case.
    pub fn normalised(mut self) -> Self {
        self.currency = self.currency.to_uppercase();
        self
    }

    pub fn with_balance(self, balance: Decimal) -> AccountWithBalance {
        let account = self.normalised();
        AccountWithBalance {
            name: account.name,
            type_: account.type_,
            currency: account.currency,
            color: account.color,
            id: account.id,
            is_archived: account.is_archived,
            balance,
        }
    }
}

/// `AccountWithBalance` — the Accounts page's shape: adds the live balance
/// (see [`account_balances`]), summed from Transaction rows rather than
/// stored. Used only by `/api/accounts`' own endpoints, never nested.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AccountWithBalance {
    pub name: String,
    #[serde(rename = "type")]
    pub type_: AccountType,
    pub currency: String,
    pub color: Option<String>,
    pub id: i32,
    pub is_archived: bool,
    pub balance: Decimal,
}

/// Query of `GET /api/accounts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AccountListQuery {
    pub include_archived: bool,
}

impl FromInput for AccountListQuery {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let include_archived = o.with_default("include_archived", || false, |v| v.bool());
        o.finish()?;
        Ok(Self { include_archived: include_archived.value() })
    }
}

/// Path parameters of `PATCH/DELETE /api/accounts/{account_id}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountPath {
    pub account_id: i64,
}

impl FromInput for AccountPath {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let account_id = o.required("account_id", |v| v.int());
        o.finish()?;
        Ok(Self { account_id: account_id.value() })
    }
}

/// One transaction row as far as balances are concerned.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct BalanceMovement {
    #[cfg_attr(feature = "sqlx", sqlx(rename = "type"))]
    pub type_: TransactionType,
    pub amount: Decimal,
    pub destination_amount: Option<Decimal>,
    pub account_id: i32,
    pub transfer_account_id: Option<i32>,
}

/// Every account's live balance (port of `_account_balances`): income adds,
/// expense subtracts, a transfer moves the source amount out and the
/// independently recorded destination amount in. Accounts without any
/// movement are absent — callers fall back to [`zero_balance`].
pub fn account_balances(movements: impl IntoIterator<Item = BalanceMovement>) -> HashMap<i32, Decimal> {
    // defaultdict(Decimal): every balance starts at Decimal("0") (exponent
    // 0), which is what decides the printed scale of the sum.
    let mut balances: HashMap<i32, Decimal> = HashMap::new();
    fn bump(balances: &mut HashMap<i32, Decimal>, id: i32, delta: &Decimal, add: bool) {
        let entry = balances.entry(id).or_insert_with(Decimal::zero);
        *entry = if add { entry.add(delta) } else { entry.sub(delta) };
    }
    for m in movements {
        match m.type_ {
            TransactionType::Income => bump(&mut balances, m.account_id, &m.amount, true),
            TransactionType::Expense => bump(&mut balances, m.account_id, &m.amount, false),
            TransactionType::Transfer => {
                bump(&mut balances, m.account_id, &m.amount, false);
                if let Some(destination) = m.transfer_account_id {
                    // `destination_amount or amount` — a zero Decimal is falsy too.
                    let incoming = match &m.destination_amount {
                        Some(d) if !d.is_zero() => d,
                        _ => &m.amount,
                    };
                    bump(&mut balances, destination, incoming, true);
                }
            }
        }
    }
    balances
}

/// `balances.get(account.id, Decimal("0"))`.
pub fn zero_balance(balances: &HashMap<i32, Decimal>, account_id: i32) -> Decimal {
    balances.get(&account_id).cloned().unwrap_or_else(Decimal::zero)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    #[test]
    fn balances_follow_python_semantics() {
        let movements = vec![
            BalanceMovement {
                type_: TransactionType::Income,
                amount: d("100.00"),
                destination_amount: None,
                account_id: 1,
                transfer_account_id: None,
            },
            BalanceMovement {
                type_: TransactionType::Transfer,
                amount: d("10.00"),
                destination_amount: Some(d("390.00")),
                account_id: 1,
                transfer_account_id: Some(2),
            },
            BalanceMovement {
                type_: TransactionType::Transfer,
                amount: d("5.00"),
                destination_amount: Some(d("0.00")),
                account_id: 1,
                transfer_account_id: Some(3),
            },
        ];
        let balances = account_balances(movements);
        assert_eq!(balances[&1].to_string(), "85.00");
        assert_eq!(balances[&2].to_string(), "390.00");
        assert_eq!(balances[&3].to_string(), "5.00");
        assert_eq!(zero_balance(&balances, 9).to_string(), "0");
    }

    #[test]
    fn create_normalises_currency() {
        let body = serde_json::json!({"name": "A", "currency": "eur"});
        let parsed = AccountCreate::from_source(&body, crate::validation::Source::Body).unwrap();
        assert_eq!(parsed.currency, "EUR");
        assert_eq!(parsed.type_, AccountType::Checking);
    }
}
