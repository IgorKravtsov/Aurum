//! API contract types for `/api/backup` — port of `backend/app/schemas/backup.py`,
//! plus the pure checks of `backend/app/services/backup_service.py`
//! (format version, referential integrity, insert order), so any client can
//! vet a backup file before uploading it.
//!
//! One type per table serves both directions: the export serialises it (field
//! order = pydantic field order) and the import validates it with
//! [`FromInput`] (pydantic's 422s). Integer columns are `i64` because Python
//! ints are unbounded on input; the server casts them on read.
//!
//! Shape of a full-database backup file (see services/backup_service.py).

mod datetime;
mod references;

use serde::{Deserialize, Serialize};

pub use references::{
    categories_in_insert_order, check_version, max_id, needs_fallback_portfolio, validate_references,
};

use crate::APP_VERSION;
use crate::decimal::Decimal;
use crate::enums::{
    AccountType, AssetClass, CapitalRole, CategoryKind, CryptoTransactionType, RecurringFrequency, RiskLevel,
    TransactionType,
};
use crate::pyfmt::PyDateTime;
use crate::validation::{FromInput, Val, ValidationErrors};

/// `BACKUP_FORMAT_VERSION` — `aurum_backup_version` of every export; an
/// import of any other version is refused.
pub const BACKUP_FORMAT_VERSION: i64 = 1;

/// Name and color of the portfolio a pre-portfolios backup's holdings are
/// filed under on restore.
pub const FALLBACK_PORTFOLIO_NAME: &str = "Main Portfolio";
pub const FALLBACK_PORTFOLIO_COLOR: &str = "#2a78d6";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct AccountBackup {
    pub id: i64,
    pub name: String,
    #[serde(rename = "type")]
    #[cfg_attr(feature = "sqlx", sqlx(rename = "type"))]
    pub type_: AccountType,
    pub currency: String,
    pub color: Option<String>,
    pub is_archived: bool,
}

impl FromInput for AccountBackup {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let id = o.required("id", |v| v.int());
        let name = o.required("name", |v| v.str());
        let type_ = o.required("type", |v| v.enum_());
        let currency = o.required("currency", |v| v.str());
        let color = o.required_nullable("color", |v| v.str());
        let is_archived = o.required("is_archived", |v| v.bool());
        o.finish()?;
        Ok(Self {
            id: id.value(),
            name: name.value(),
            type_: type_.value(),
            currency: currency.value(),
            color: color.value(),
            is_archived: is_archived.value(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct CategoryBackup {
    pub id: i64,
    pub name: String,
    pub kind: CategoryKind,
    pub icon: Option<String>,
    pub color: String,
    pub sort_order: i64,
    pub is_default: bool,
    // Defaulted so a backup exported before subcategories existed still
    // imports cleanly under the same format version.
    pub parent_id: Option<i64>,
}

impl FromInput for CategoryBackup {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let id = o.required("id", |v| v.int());
        let name = o.required("name", |v| v.str());
        let kind = o.required("kind", |v| v.enum_());
        let icon = o.required_nullable("icon", |v| v.str());
        let color = o.required("color", |v| v.str());
        let sort_order = o.required("sort_order", |v| v.int());
        let is_default = o.required("is_default", |v| v.bool());
        let parent_id = o.optional("parent_id", |v| v.int());
        o.finish()?;
        Ok(Self {
            id: id.value(),
            name: name.value(),
            kind: kind.value(),
            icon: icon.value(),
            color: color.value(),
            sort_order: sort_order.value(),
            is_default: is_default.value(),
            parent_id: parent_id.value(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct TagBackup {
    pub id: i64,
    pub name: String,
}

impl FromInput for TagBackup {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let id = o.required("id", |v| v.int());
        let name = o.required("name", |v| v.str());
        o.finish()?;
        Ok(Self { id: id.value(), name: name.value() })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct FxRateBackup {
    pub id: i64,
    pub currency: String,
    pub requested_date: chrono::NaiveDate,
    pub effective_date: chrono::NaiveDate,
    pub rate_to_pln: Decimal,
    pub provider: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub fetched_at: PyDateTime,
}

impl FromInput for FxRateBackup {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let id = o.required("id", |v| v.int());
        let currency = o.required("currency", |v| v.str());
        let requested_date = o.required("requested_date", |v| v.date());
        let effective_date = o.required("effective_date", |v| v.date());
        let rate_to_pln = o.required("rate_to_pln", |v| v.decimal());
        let provider = o.required("provider", |v| v.str());
        let fetched_at = o.required("fetched_at", |v| datetime::datetime(&v));
        o.finish()?;
        Ok(Self {
            id: id.value(),
            currency: currency.value(),
            requested_date: requested_date.value(),
            effective_date: effective_date.value(),
            rate_to_pln: rate_to_pln.value(),
            provider: provider.value(),
            fetched_at: fetched_at.value(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct TransactionBackup {
    pub id: i64,
    pub account_id: i64,
    pub category_id: Option<i64>,
    pub transfer_account_id: Option<i64>,
    #[serde(rename = "type")]
    #[cfg_attr(feature = "sqlx", sqlx(rename = "type"))]
    pub type_: TransactionType,
    pub amount: Decimal,
    // Default keeps backups from before cross-currency transfers importable.
    pub destination_amount: Option<Decimal>,
    pub description: String,
    pub merchant: Option<String>,
    pub notes: Option<String>,
    pub date: chrono::NaiveDate,
    // Defaulted so a backup exported before tags existed still imports
    // cleanly under the same format version. Not a plain column — filled in
    // by the export from the `transaction_tags` association table.
    #[cfg_attr(feature = "sqlx", sqlx(skip))]
    pub tag_ids: Vec<i64>,
}

impl FromInput for TransactionBackup {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let id = o.required("id", |v| v.int());
        let account_id = o.required("account_id", |v| v.int());
        let category_id = o.required_nullable("category_id", |v| v.int());
        let transfer_account_id = o.required_nullable("transfer_account_id", |v| v.int());
        let type_ = o.required("type", |v| v.enum_());
        let amount = o.required("amount", |v| v.decimal());
        let destination_amount = o.optional("destination_amount", |v| v.decimal());
        let description = o.required("description", |v| v.str());
        let merchant = o.required_nullable("merchant", |v| v.str());
        let notes = o.required_nullable("notes", |v| v.str());
        let date = o.required("date", |v| v.date());
        let tag_ids = o.with_default("tag_ids", Vec::new, |v| v.list(|v| v.int()));
        o.finish()?;
        Ok(Self {
            id: id.value(),
            account_id: account_id.value(),
            category_id: category_id.value(),
            transfer_account_id: transfer_account_id.value(),
            type_: type_.value(),
            amount: amount.value(),
            destination_amount: destination_amount.value(),
            description: description.value(),
            merchant: merchant.value(),
            notes: notes.value(),
            date: date.value(),
            tag_ids: tag_ids.value(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct TransactionSplitBackup {
    pub id: i64,
    pub transaction_id: i64,
    pub category_id: Option<i64>,
    pub amount: Decimal,
    pub note: Option<String>,
}

impl FromInput for TransactionSplitBackup {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let id = o.required("id", |v| v.int());
        let transaction_id = o.required("transaction_id", |v| v.int());
        let category_id = o.required_nullable("category_id", |v| v.int());
        let amount = o.required("amount", |v| v.decimal());
        let note = o.required_nullable("note", |v| v.str());
        o.finish()?;
        Ok(Self {
            id: id.value(),
            transaction_id: transaction_id.value(),
            category_id: category_id.value(),
            amount: amount.value(),
            note: note.value(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct AssetBackup {
    pub id: i64,
    pub name: String,
    pub asset_class: AssetClass,
    pub currency: String,
    pub notes: Option<String>,
    // Defaulted so a backup exported before capital_role/monthly_cash_flow
    // existed still imports cleanly under the same format version.
    pub capital_role: CapitalRole,
    pub monthly_cash_flow: Option<Decimal>,
    // Defaulted so a backup exported before risk_level existed still
    // imports cleanly under the same format version.
    pub risk_level: RiskLevel,
}

impl FromInput for AssetBackup {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let id = o.required("id", |v| v.int());
        let name = o.required("name", |v| v.str());
        let asset_class = o.required("asset_class", |v| v.enum_());
        let currency = o.required("currency", |v| v.str());
        let notes = o.required_nullable("notes", |v| v.str());
        let capital_role = o.with_default("capital_role", || CapitalRole::Neutral, |v| v.enum_());
        let monthly_cash_flow = o.optional("monthly_cash_flow", |v| v.decimal());
        let risk_level = o.with_default("risk_level", || RiskLevel::Medium, |v| v.enum_());
        o.finish()?;
        Ok(Self {
            id: id.value(),
            name: name.value(),
            asset_class: asset_class.value(),
            currency: currency.value(),
            notes: notes.value(),
            capital_role: capital_role.value(),
            monthly_cash_flow: monthly_cash_flow.value(),
            risk_level: risk_level.value(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct AssetValuationBackup {
    pub id: i64,
    pub asset_id: i64,
    pub value: Decimal,
    pub as_of_date: chrono::NaiveDate,
}

impl FromInput for AssetValuationBackup {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let id = o.required("id", |v| v.int());
        let asset_id = o.required("asset_id", |v| v.int());
        let value = o.required("value", |v| v.decimal());
        let as_of_date = o.required("as_of_date", |v| v.date());
        o.finish()?;
        Ok(Self { id: id.value(), asset_id: asset_id.value(), value: value.value(), as_of_date: as_of_date.value() })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct CryptoPortfolioBackup {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub is_archived: bool,
}

impl FromInput for CryptoPortfolioBackup {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let id = o.required("id", |v| v.int());
        let name = o.required("name", |v| v.str());
        let color = o.required_nullable("color", |v| v.str());
        let is_archived = o.required("is_archived", |v| v.bool());
        o.finish()?;
        Ok(Self { id: id.value(), name: name.value(), color: color.value(), is_archived: is_archived.value() })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct CryptoHoldingBackup {
    // asset_id doubles as this row's own primary key (see
    // models/crypto.py's CryptoHolding) — there's no separate `id`. No
    // quantity here — it's derived from crypto_transactions below, not
    // stored.
    pub asset_id: i64,
    // Defaulted so a backup exported before crypto portfolios existed still
    // imports cleanly under the same format version — the restore resolves
    // a missing portfolio_id to an auto-created fallback portfolio (see
    // FALLBACK_PORTFOLIO_NAME).
    pub portfolio_id: Option<i64>,
    pub coingecko_id: String,
    pub symbol: String,
    pub name: String,
    pub thumb_url: Option<String>,
    pub last_price: Option<Decimal>,
    pub price_change_1h: Option<Decimal>,
    pub price_change_24h: Option<Decimal>,
    pub price_change_7d: Option<Decimal>,
    // Defaulted so a backup exported before 30d/1y existed still imports
    // cleanly under the same format version.
    pub price_change_30d: Option<Decimal>,
    pub price_change_1y: Option<Decimal>,
}

impl FromInput for CryptoHoldingBackup {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let asset_id = o.required("asset_id", |v| v.int());
        let portfolio_id = o.optional("portfolio_id", |v| v.int());
        let coingecko_id = o.required("coingecko_id", |v| v.str());
        let symbol = o.required("symbol", |v| v.str());
        let name = o.required("name", |v| v.str());
        let thumb_url = o.required_nullable("thumb_url", |v| v.str());
        let last_price = o.optional("last_price", |v| v.decimal());
        let price_change_1h = o.optional("price_change_1h", |v| v.decimal());
        let price_change_24h = o.optional("price_change_24h", |v| v.decimal());
        let price_change_7d = o.optional("price_change_7d", |v| v.decimal());
        let price_change_30d = o.optional("price_change_30d", |v| v.decimal());
        let price_change_1y = o.optional("price_change_1y", |v| v.decimal());
        o.finish()?;
        Ok(Self {
            asset_id: asset_id.value(),
            portfolio_id: portfolio_id.value(),
            coingecko_id: coingecko_id.value(),
            symbol: symbol.value(),
            name: name.value(),
            thumb_url: thumb_url.value(),
            last_price: last_price.value(),
            price_change_1h: price_change_1h.value(),
            price_change_24h: price_change_24h.value(),
            price_change_7d: price_change_7d.value(),
            price_change_30d: price_change_30d.value(),
            price_change_1y: price_change_1y.value(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct CryptoTransactionBackup {
    pub id: i64,
    pub asset_id: i64,
    #[serde(rename = "type")]
    #[cfg_attr(feature = "sqlx", sqlx(rename = "type"))]
    pub type_: CryptoTransactionType,
    pub quantity: Decimal,
    pub price_per_unit: Decimal,
    pub date: chrono::NaiveDate,
    pub note: Option<String>,
}

impl FromInput for CryptoTransactionBackup {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let id = o.required("id", |v| v.int());
        let asset_id = o.required("asset_id", |v| v.int());
        let type_ = o.required("type", |v| v.enum_());
        let quantity = o.required("quantity", |v| v.decimal());
        let price_per_unit = o.required("price_per_unit", |v| v.decimal());
        let date = o.required("date", |v| v.date());
        let note = o.required_nullable("note", |v| v.str());
        o.finish()?;
        Ok(Self {
            id: id.value(),
            asset_id: asset_id.value(),
            type_: type_.value(),
            quantity: quantity.value(),
            price_per_unit: price_per_unit.value(),
            date: date.value(),
            note: note.value(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct BudgetBackup {
    pub id: i64,
    pub category_id: i64,
    pub monthly_limit: Decimal,
}

impl FromInput for BudgetBackup {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let id = o.required("id", |v| v.int());
        let category_id = o.required("category_id", |v| v.int());
        let monthly_limit = o.required("monthly_limit", |v| v.decimal());
        o.finish()?;
        Ok(Self { id: id.value(), category_id: category_id.value(), monthly_limit: monthly_limit.value() })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct GoalBackup {
    pub id: i64,
    pub name: String,
    pub target_amount: Decimal,
    pub target_date: Option<chrono::NaiveDate>,
}

impl FromInput for GoalBackup {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let id = o.required("id", |v| v.int());
        let name = o.required("name", |v| v.str());
        let target_amount = o.required("target_amount", |v| v.decimal());
        let target_date = o.required_nullable("target_date", |v| v.date());
        o.finish()?;
        Ok(Self {
            id: id.value(),
            name: name.value(),
            target_amount: target_amount.value(),
            target_date: target_date.value(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct GoalContributionBackup {
    pub id: i64,
    pub goal_id: i64,
    pub amount: Decimal,
    pub date: chrono::NaiveDate,
    pub note: Option<String>,
}

impl FromInput for GoalContributionBackup {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let id = o.required("id", |v| v.int());
        let goal_id = o.required("goal_id", |v| v.int());
        let amount = o.required("amount", |v| v.decimal());
        let date = o.required("date", |v| v.date());
        let note = o.required_nullable("note", |v| v.str());
        o.finish()?;
        Ok(Self {
            id: id.value(),
            goal_id: goal_id.value(),
            amount: amount.value(),
            date: date.value(),
            note: note.value(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct RecurringTransactionBackup {
    pub id: i64,
    pub account_id: i64,
    pub category_id: Option<i64>,
    pub transfer_account_id: Option<i64>,
    #[serde(rename = "type")]
    #[cfg_attr(feature = "sqlx", sqlx(rename = "type"))]
    pub type_: TransactionType,
    pub amount: Decimal,
    // Default keeps backups from before cross-currency transfers importable.
    pub destination_amount: Option<Decimal>,
    pub description: String,
    pub merchant: Option<String>,
    pub notes: Option<String>,
    pub frequency: RecurringFrequency,
    pub anchor_date: chrono::NaiveDate,
    pub last_posted_date: Option<chrono::NaiveDate>,
    pub is_active: bool,
}

impl FromInput for RecurringTransactionBackup {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let id = o.required("id", |v| v.int());
        let account_id = o.required("account_id", |v| v.int());
        let category_id = o.required_nullable("category_id", |v| v.int());
        let transfer_account_id = o.required_nullable("transfer_account_id", |v| v.int());
        let type_ = o.required("type", |v| v.enum_());
        let amount = o.required("amount", |v| v.decimal());
        let destination_amount = o.optional("destination_amount", |v| v.decimal());
        let description = o.required("description", |v| v.str());
        let merchant = o.required_nullable("merchant", |v| v.str());
        let notes = o.required_nullable("notes", |v| v.str());
        let frequency = o.required("frequency", |v| v.enum_());
        let anchor_date = o.required("anchor_date", |v| v.date());
        let last_posted_date = o.required_nullable("last_posted_date", |v| v.date());
        let is_active = o.required("is_active", |v| v.bool());
        o.finish()?;
        Ok(Self {
            id: id.value(),
            account_id: account_id.value(),
            category_id: category_id.value(),
            transfer_account_id: transfer_account_id.value(),
            type_: type_.value(),
            amount: amount.value(),
            destination_amount: destination_amount.value(),
            description: description.value(),
            merchant: merchant.value(),
            notes: notes.value(),
            frequency: frequency.value(),
            anchor_date: anchor_date.value(),
            last_posted_date: last_posted_date.value(),
            is_active: is_active.value(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct AppSettingsBackup {
    pub currency: String,
    // Defaulted so a backup exported before these thresholds existed still
    // imports cleanly under the same format version.
    pub negative_cash_flow_threshold_months: i64,
    pub net_worth_decline_threshold_months: i64,
    // Defaulted so a backup exported before this threshold existed still
    // imports cleanly under the same format version.
    pub risky_allocation_threshold_percent: i64,
    // Defaulted so a backup exported before idle-cash thresholds existed
    // still imports cleanly under the same format version.
    pub idle_cash_threshold_amount: Decimal,
    pub idle_cash_threshold_days: i64,
}

impl AppSettingsBackup {
    /// `AppSettingsBackup(currency=currency)` — every threshold at its default.
    pub fn with_currency(currency: &str) -> Self {
        Self {
            currency: currency.to_string(),
            negative_cash_flow_threshold_months: 2,
            net_worth_decline_threshold_months: 2,
            risky_allocation_threshold_percent: 20,
            idle_cash_threshold_amount: default_idle_cash_amount(),
            idle_cash_threshold_days: 60,
        }
    }
}

/// `Decimal("1000")` — printed without decimals, unlike the stored column.
fn default_idle_cash_amount() -> Decimal {
    "1000".parse().expect("valid decimal literal")
}

impl FromInput for AppSettingsBackup {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let currency = o.required("currency", |v| v.str());
        let negative = o.with_default("negative_cash_flow_threshold_months", || 2, |v| v.int());
        let decline = o.with_default("net_worth_decline_threshold_months", || 2, |v| v.int());
        let risky = o.with_default("risky_allocation_threshold_percent", || 20, |v| v.int());
        let amount = o.with_default("idle_cash_threshold_amount", default_idle_cash_amount, |v| v.decimal());
        let days = o.with_default("idle_cash_threshold_days", || 60, |v| v.int());
        o.finish()?;
        Ok(Self {
            currency: currency.value(),
            negative_cash_flow_threshold_months: negative.value(),
            net_worth_decline_threshold_months: decline.value(),
            risky_allocation_threshold_percent: risky.value(),
            idle_cash_threshold_amount: amount.value(),
            idle_cash_threshold_days: days.value(),
        })
    }
}

/// `BackupPayload` — a full, portable snapshot of every table.
/// `aurum_backup_version` is checked on import so an incompatible/future
/// file is rejected cleanly instead of half-applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BackupPayload {
    pub aurum_backup_version: i64,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub exported_at: PyDateTime,
    pub app_version: String,
    pub accounts: Vec<AccountBackup>,
    pub categories: Vec<CategoryBackup>,
    // Defaulted so a backup exported before tags existed still imports
    // cleanly under the same format version.
    pub tags: Vec<TagBackup>,
    pub transactions: Vec<TransactionBackup>,
    // Defaulted so a backup exported before transaction splitting existed
    // still imports cleanly under the same format version.
    pub transaction_splits: Vec<TransactionSplitBackup>,
    pub assets: Vec<AssetBackup>,
    pub asset_valuations: Vec<AssetValuationBackup>,
    // Defaulted so a backup exported before crypto holdings/portfolios
    // existed still imports cleanly under the same format version.
    pub crypto_portfolios: Vec<CryptoPortfolioBackup>,
    pub crypto_holdings: Vec<CryptoHoldingBackup>,
    pub crypto_transactions: Vec<CryptoTransactionBackup>,
    // Defaulted so a backup exported before budgets existed still imports
    // cleanly under the same format version.
    pub budgets: Vec<BudgetBackup>,
    // Defaulted so a backup exported before goals existed still imports
    // cleanly under the same format version.
    pub goals: Vec<GoalBackup>,
    pub goal_contributions: Vec<GoalContributionBackup>,
    // Defaulted so a backup exported before recurring transactions existed
    // still imports cleanly under the same format version.
    pub recurring_transactions: Vec<RecurringTransactionBackup>,
    // Derived but persisted so restored historical reports still work when
    // the rate provider is offline. Older backups simply refill this cache.
    pub fx_rates: Vec<FxRateBackup>,
    // Defaulted so a backup exported before the currency setting existed
    // still imports cleanly under the same format version.
    pub app_settings: AppSettingsBackup,
}

impl BackupPayload {
    /// An export of the current format stamped with this build's version;
    /// the tables are filled in by the caller.
    pub fn new_export(exported_at: PyDateTime) -> Self {
        Self {
            aurum_backup_version: BACKUP_FORMAT_VERSION,
            exported_at,
            app_version: APP_VERSION.to_string(),
            accounts: Vec::new(),
            categories: Vec::new(),
            tags: Vec::new(),
            transactions: Vec::new(),
            transaction_splits: Vec::new(),
            assets: Vec::new(),
            asset_valuations: Vec::new(),
            crypto_portfolios: Vec::new(),
            crypto_holdings: Vec::new(),
            crypto_transactions: Vec::new(),
            budgets: Vec::new(),
            goals: Vec::new(),
            goal_contributions: Vec::new(),
            recurring_transactions: Vec::new(),
            fx_rates: Vec::new(),
            app_settings: AppSettingsBackup::with_currency("USD"),
        }
    }
}

impl FromInput for BackupPayload {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let version = o.required("aurum_backup_version", |v| v.int());
        let exported_at = o.required("exported_at", |v| datetime::datetime(&v));
        let app_version = o.required("app_version", |v| v.str());
        let accounts = o.required("accounts", |v| v.list(|v| v.model()));
        let categories = o.required("categories", |v| v.list(|v| v.model()));
        let tags = o.with_default("tags", Vec::new, |v| v.list(|v| v.model()));
        let transactions = o.required("transactions", |v| v.list(|v| v.model()));
        let transaction_splits = o.with_default("transaction_splits", Vec::new, |v| v.list(|v| v.model()));
        let assets = o.required("assets", |v| v.list(|v| v.model()));
        let asset_valuations = o.required("asset_valuations", |v| v.list(|v| v.model()));
        let crypto_portfolios = o.with_default("crypto_portfolios", Vec::new, |v| v.list(|v| v.model()));
        let crypto_holdings = o.with_default("crypto_holdings", Vec::new, |v| v.list(|v| v.model()));
        let crypto_transactions = o.with_default("crypto_transactions", Vec::new, |v| v.list(|v| v.model()));
        let budgets = o.with_default("budgets", Vec::new, |v| v.list(|v| v.model()));
        let goals = o.with_default("goals", Vec::new, |v| v.list(|v| v.model()));
        let goal_contributions = o.with_default("goal_contributions", Vec::new, |v| v.list(|v| v.model()));
        let recurring = o.with_default("recurring_transactions", Vec::new, |v| v.list(|v| v.model()));
        let fx_rates = o.with_default("fx_rates", Vec::new, |v| v.list(|v| v.model()));
        let app_settings = o.with_default("app_settings", || AppSettingsBackup::with_currency("USD"), |v| v.model());
        o.finish()?;
        Ok(Self {
            aurum_backup_version: version.value(),
            exported_at: exported_at.value(),
            app_version: app_version.value(),
            accounts: accounts.value(),
            categories: categories.value(),
            tags: tags.value(),
            transactions: transactions.value(),
            transaction_splits: transaction_splits.value(),
            assets: assets.value(),
            asset_valuations: asset_valuations.value(),
            crypto_portfolios: crypto_portfolios.value(),
            crypto_holdings: crypto_holdings.value(),
            crypto_transactions: crypto_transactions.value(),
            budgets: budgets.value(),
            goals: goals.value(),
            goal_contributions: goal_contributions.value(),
            recurring_transactions: recurring.value(),
            fx_rates: fx_rates.value(),
            app_settings: app_settings.value(),
        })
    }
}

/// Response of `POST /api/backup/import`: `{"status": "ok"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ImportResult {
    pub status: String,
}

impl ImportResult {
    pub fn ok() -> Self {
        Self { status: "ok".to_string() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::Source;
    use serde_json::json;

    fn minimal() -> serde_json::Value {
        json!({
            "aurum_backup_version": 1,
            "exported_at": "2026-01-02T03:04:05.000120Z",
            "app_version": "1.3.2",
            "accounts": [],
            "categories": [],
            "transactions": [],
            "assets": [],
            "asset_valuations": []
        })
    }

    #[test]
    fn defaults_fill_in_like_pydantic() {
        let payload = BackupPayload::from_source(&minimal(), Source::Body).ok().unwrap();
        assert_eq!(payload.app_settings, AppSettingsBackup::with_currency("USD"));
        assert_eq!(payload.app_settings.idle_cash_threshold_amount.to_string(), "1000");
        assert_eq!(payload.exported_at.to_string(), "2026-01-02T03:04:05.000120Z");
        assert!(payload.tags.is_empty() && payload.fx_rates.is_empty());
    }

    #[test]
    fn export_serialises_in_pydantic_field_order() {
        let mut payload = BackupPayload::new_export(PyDateTime("2026-01-02T03:04:05Z".parse().unwrap()));
        payload.transactions.push(TransactionBackup {
            id: 1,
            account_id: 2,
            category_id: None,
            transfer_account_id: None,
            type_: TransactionType::Expense,
            amount: "10.00".parse().unwrap(),
            destination_amount: None,
            description: "x".into(),
            merchant: None,
            notes: None,
            date: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            tag_ids: vec![3, 1],
        });
        let text = serde_json::to_string(&payload).unwrap();
        assert!(text.starts_with(
            r#"{"aurum_backup_version":1,"exported_at":"2026-01-02T03:04:05Z","app_version":"1.3.2","accounts":[],"categories":[],"tags":[],"transactions":[{"id":1,"account_id":2,"category_id":null,"transfer_account_id":null,"type":"expense","amount":"10.00","destination_amount":null,"description":"x","merchant":null,"notes":null,"date":"2026-01-01","tag_ids":[3,1]}],"transaction_splits":[]"#
        ));
        assert!(text.ends_with(
            r#""app_settings":{"currency":"USD","negative_cash_flow_threshold_months":2,"net_worth_decline_threshold_months":2,"risky_allocation_threshold_percent":20,"idle_cash_threshold_amount":"1000","idle_cash_threshold_days":60}}"#
        ));
    }

    #[test]
    fn nested_errors_carry_list_index() {
        let mut body = minimal();
        body["accounts"] = json!([{"id": 1, "name": "A", "type": "nope", "currency": "USD", "is_archived": false}]);
        let errors = BackupPayload::from_source(&body, Source::Body).err().unwrap();
        let text = serde_json::to_string(&errors.0).unwrap();
        assert!(text.contains(r#""loc":["body","accounts",0,"type"]"#), "{text}");
        assert!(text.contains(r#""loc":["body","accounts",0,"color"]"#), "{text}");
    }
}
