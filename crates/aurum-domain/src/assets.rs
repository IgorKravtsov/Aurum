//! API contract types for `/api/assets` — port of the former Python backend's `app/schemas/asset.py`,
//! plus the pure `_to_read` projection of the former Python backend's `app/api/routes/assets.py`.
//!
//! An Asset is a manually-tracked net-worth item (investments, crypto,
//! property, ...). Cash is intentionally NOT an Asset — it's derived live
//! from Account/Transaction balances (see `net_worth`). Each asset's value
//! history lives in `asset_valuations`; the latest row (by `as_of_date`) is
//! its current value.
//!
//! Follows `crates/aurum-server/PORTING.md`: response DTOs derive
//! `Serialize` (field order = pydantic field order, inherited fields first),
//! request DTOs implement `validation::FromInput`.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::accounts::CURRENCY_PATTERN;
use crate::decimal::Decimal;
use crate::enums::{AssetClass, CapitalRole, RiskLevel};
use crate::validation::{FromInput, Patch, Res, Val, ValidationErrors};

/// `value: Decimal = Field(ge=0, max_digits=14, decimal_places=2)`.
fn valuation_value<'a>(v: Val<'a>) -> Res<'a, Decimal> {
    v.decimal()?.digits(Some(14), Some(2))?.ge("0")
}

/// `as_of_date: date = Field(default_factory=date.today)` — `None` means
/// "absent, use today", which the server fills in (no clock in this crate).
/// An explicit `null` is still a `date_type` error, like pydantic.
fn as_of_date<'a>(v: Val<'a>) -> Res<'a, Option<NaiveDate>> {
    Ok(v.date()?.map(Some))
}

/// `currency: str = Field(min_length=3, max_length=3, pattern=...)` plus the
/// `_normalise_currency` field validator (upper-case).
fn currency<'a>(v: Val<'a>) -> Res<'a, String> {
    Ok(v.str()?.min_len(3)?.max_len(3)?.pattern(CURRENCY_PATTERN)?.map(|s| s.to_uppercase()))
}

/// `AssetValuationCreate` — body of `POST /api/assets/{asset_id}/valuations`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AssetValuationCreate {
    pub value: Decimal,
    /// `None` → today (filled in by the server).
    pub as_of_date: Option<NaiveDate>,
}

impl FromInput for AssetValuationCreate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let value = o.required("value", valuation_value);
        let as_of = o.with_default("as_of_date", || None, as_of_date);
        o.finish()?;
        Ok(Self { value: value.value(), as_of_date: as_of.value() })
    }
}

/// `AssetValuationRead`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct AssetValuationRead {
    pub id: i32,
    pub value: Decimal,
    pub as_of_date: NaiveDate,
}

/// `AssetCreate` (= `AssetBase` + the first valuation) — body of `POST /api/assets`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AssetCreate {
    pub name: String,
    pub asset_class: AssetClass,
    pub currency: String,
    pub notes: Option<String>,
    pub capital_role: CapitalRole,
    // Rough self-reported monthly net cash flow — informational, not tracked
    // transactions (see Asset.monthly_cash_flow).
    pub monthly_cash_flow: Option<Decimal>,
    pub risk_level: RiskLevel,
    pub value: Decimal,
    /// `None` → today (filled in by the server).
    pub as_of_date: Option<NaiveDate>,
}

impl FromInput for AssetCreate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let name = o.required("name", |v| v.str()?.min_len(1)?.max_len(150));
        let asset_class = o.required("asset_class", |v| v.enum_());
        // Like any pydantic field validator, `_normalise_currency` doesn't
        // run on the default.
        let currency = o.with_default("currency", || "USD".to_string(), currency);
        let notes = o.optional("notes", |v| v.str()?.max_len(2000));
        let capital_role = o.with_default("capital_role", || CapitalRole::Neutral, |v| v.enum_());
        let monthly_cash_flow = o.optional("monthly_cash_flow", |v| v.decimal()?.digits(Some(14), Some(2)));
        let risk_level = o.with_default("risk_level", || RiskLevel::Medium, |v| v.enum_());
        let value = o.required("value", valuation_value);
        let as_of = o.with_default("as_of_date", || None, as_of_date);
        o.finish()?;
        Ok(Self {
            name: name.value(),
            asset_class: asset_class.value(),
            currency: currency.value(),
            notes: notes.value(),
            capital_role: capital_role.value(),
            monthly_cash_flow: monthly_cash_flow.value(),
            risk_level: risk_level.value(),
            value: value.value(),
            as_of_date: as_of.value(),
        })
    }
}

/// `AssetUpdate` — partial update (`exclude_unset`). An explicit `null` for
/// a NOT NULL column is written as NULL and fails at commit (500), as under
/// Python.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AssetUpdate {
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub name: Patch<String>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<AssetClass>))]
    pub asset_class: Patch<AssetClass>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub currency: Patch<String>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub notes: Patch<String>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<CapitalRole>))]
    pub capital_role: Patch<CapitalRole>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub monthly_cash_flow: Patch<Decimal>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<RiskLevel>))]
    pub risk_level: Patch<RiskLevel>,
}

impl FromInput for AssetUpdate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let name = o.patch("name", |v| v.str()?.min_len(1)?.max_len(150));
        let asset_class = o.patch("asset_class", |v| v.enum_());
        let currency = o.patch("currency", currency);
        let notes = o.patch("notes", |v| v.str()?.max_len(2000));
        let capital_role = o.patch("capital_role", |v| v.enum_());
        let monthly_cash_flow = o.patch("monthly_cash_flow", |v| v.decimal()?.digits(Some(14), Some(2)));
        let risk_level = o.patch("risk_level", |v| v.enum_());
        o.finish()?;
        Ok(Self {
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

/// `AssetRead` (= `AssetBase` + id, current value and its date).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AssetRead {
    pub name: String,
    pub asset_class: AssetClass,
    pub currency: String,
    pub notes: Option<String>,
    pub capital_role: CapitalRole,
    pub monthly_cash_flow: Option<Decimal>,
    pub risk_level: RiskLevel,
    pub id: i32,
    pub current_value: Decimal,
    pub as_of_date: NaiveDate,
}

/// One asset row joined with its latest valuation (if any) — what
/// `_to_read` looks at.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct AssetRow {
    pub id: i32,
    pub name: String,
    pub asset_class: AssetClass,
    pub currency: String,
    pub notes: Option<String>,
    pub capital_role: CapitalRole,
    pub monthly_cash_flow: Option<Decimal>,
    pub risk_level: RiskLevel,
    /// `created_at.date()` (UTC).
    pub created_date: NaiveDate,
    pub latest_value: Option<Decimal>,
    pub latest_date: Option<NaiveDate>,
}

impl AssetRow {
    /// `_to_read`: the latest valuation is the current value; an asset
    /// without any valuation reads as `0` as of its creation date. pydantic
    /// re-runs `_normalise_currency` on the way out, so the currency is
    /// always upper-case.
    pub fn into_read(self) -> AssetRead {
        let (current_value, as_of_date) = match (self.latest_value, self.latest_date) {
            (Some(value), Some(date)) => (value, date),
            _ => (Decimal::zero(), self.created_date),
        };
        AssetRead {
            name: self.name,
            asset_class: self.asset_class,
            currency: self.currency.to_uppercase(),
            notes: self.notes,
            capital_role: self.capital_role,
            monthly_cash_flow: self.monthly_cash_flow,
            risk_level: self.risk_level,
            id: self.id,
            current_value,
            as_of_date,
        }
    }
}

/// Path parameters of `/api/assets/{asset_id}[...]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetPath {
    pub asset_id: i64,
}

impl FromInput for AssetPath {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let asset_id = o.required("asset_id", |v| v.int());
        o.finish()?;
        Ok(Self { asset_id: asset_id.value() })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::validation::Source;

    #[test]
    fn create_applies_defaults_and_normalises_currency() {
        let body = json!({"name": "Flat", "asset_class": "real_estate", "currency": "eur", "value": "1000"});
        let asset = AssetCreate::from_source(&body, Source::Body).unwrap();
        assert_eq!(asset.currency, "EUR");
        assert_eq!(asset.capital_role, CapitalRole::Neutral);
        assert_eq!(asset.risk_level, RiskLevel::Medium);
        assert_eq!(asset.as_of_date, None);
        assert_eq!(asset.value.to_string(), "1000");
    }

    #[test]
    fn create_rejects_null_date_and_negative_value() {
        let body = json!({"name": "Flat", "asset_class": "other", "value": "-1", "as_of_date": null});
        let errors = AssetCreate::from_source(&body, Source::Body).unwrap_err();
        assert_eq!(errors.0.len(), 2);
    }

    #[test]
    fn asset_without_valuations_reads_zero() {
        let row = AssetRow {
            id: 1,
            name: "Car".into(),
            asset_class: AssetClass::Vehicles,
            currency: "usd".into(),
            notes: None,
            capital_role: CapitalRole::Drain,
            monthly_cash_flow: None,
            risk_level: RiskLevel::High,
            created_date: NaiveDate::from_ymd_opt(2026, 1, 2).unwrap(),
            latest_value: None,
            latest_date: None,
        };
        let read = row.into_read();
        assert_eq!(read.current_value.to_string(), "0");
        assert_eq!(read.currency, "USD");
        assert_eq!(read.as_of_date, NaiveDate::from_ymd_opt(2026, 1, 2).unwrap());
    }
}
