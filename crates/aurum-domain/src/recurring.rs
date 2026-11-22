//! API contract types for `/api/recurring` — port of the former Python backend's `app/schemas/recurring.py`,
//! plus the pure half of the former Python backend's `app/services/recurring_service.py`: the
//! schedule ([`advance`], [`next_due_date`]), the write-time rules
//! ([`category_kind_violation`], [`normalise_destination_amount`]) and the
//! read model ([`recurring_read`]).
//!
//! Follows `crates/aurum-server/PORTING.md`: response DTOs derive
//! `Serialize` (field order = pydantic field order, inherited fields first),
//! request DTOs implement `validation::FromInput`.

use std::collections::HashMap;

use chrono::{Datelike, Days, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::categories::CategoryRead;
use crate::decimal::Decimal;
use crate::enums::{RecurringFrequency, TransactionType};
use crate::text::capitalize_first_letter;
use crate::transactions::category_kind_for;
use crate::validation::{FromInput, Patch, Res, Val, ValidationErrors};

// ---------------------------------------------------------------------------
// Request DTOs
// ---------------------------------------------------------------------------

fn description_field<'a>(v: Val<'a>) -> Res<'a, String> {
    // Keeps templates consistent with regular transactions — see
    // transactions.rs for the same rule.
    Ok(v.str()?.min_len(1)?.max_len(255)?.map(|s| capitalize_first_letter(&s)))
}

/// `RecurringTransactionCreate` — body of `POST /api/recurring`.
///
/// Unlike transactions, amounts carry no `max_digits`/`decimal_places`
/// (the `numeric(14,2)` column rounds or rejects them) and `merchant` has no
/// length limit here (a too-long one fails in the database, 500).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RecurringTransactionCreate {
    pub account_id: i64,
    pub category_id: Option<i64>,
    pub transfer_account_id: Option<i64>,
    #[serde(rename = "type")]
    pub type_: TransactionType,
    pub amount: Decimal,
    pub destination_amount: Option<Decimal>,
    pub description: String,
    pub merchant: Option<String>,
    pub notes: Option<String>,
    pub frequency: RecurringFrequency,
    pub anchor_date: NaiveDate,
}

impl FromInput for RecurringTransactionCreate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let account_id = o.required("account_id", |v| v.int());
        let category_id = o.optional("category_id", |v| v.int());
        let transfer_account_id = o.optional("transfer_account_id", |v| v.int());
        let type_ = o.required("type", |v| v.enum_());
        let amount = o.required("amount", |v| v.decimal()?.gt("0"));
        let destination_amount = o.optional("destination_amount", |v| v.decimal()?.gt("0"));
        let description = o.required("description", description_field);
        let merchant = o.optional("merchant", |v| v.str());
        let notes = o.optional("notes", |v| v.str()?.max_len(2000));
        let frequency = o.required("frequency", |v| v.enum_());
        let anchor_date = o.required("anchor_date", |v| v.date());
        o.finish()?;
        Ok(Self {
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
        })
    }
}

/// `RecurringTransactionUpdate` — partial update (`exclude_unset`). An
/// explicit `null` for a NOT NULL column passes validation and fails in the
/// database (500), like the Python commit.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RecurringTransactionUpdate {
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>))]
    pub account_id: Patch<i64>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>))]
    pub category_id: Patch<i64>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>))]
    pub transfer_account_id: Patch<i64>,
    #[serde(rename = "type", skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<TransactionType>))]
    pub type_: Patch<TransactionType>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub amount: Patch<Decimal>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub destination_amount: Patch<Decimal>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub description: Patch<String>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub merchant: Patch<String>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub notes: Patch<String>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<RecurringFrequency>))]
    pub frequency: Patch<RecurringFrequency>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<NaiveDate>))]
    pub anchor_date: Patch<NaiveDate>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<bool>))]
    pub is_active: Patch<bool>,
}

impl FromInput for RecurringTransactionUpdate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let account_id = o.patch("account_id", |v| v.int());
        let category_id = o.patch("category_id", |v| v.int());
        let transfer_account_id = o.patch("transfer_account_id", |v| v.int());
        let type_ = o.patch("type", |v| v.enum_());
        let amount = o.patch("amount", |v| v.decimal()?.gt("0"));
        let destination_amount = o.patch("destination_amount", |v| v.decimal()?.gt("0"));
        let description = o.patch("description", description_field);
        let merchant = o.patch("merchant", |v| v.str());
        let notes = o.patch("notes", |v| v.str()?.max_len(2000));
        let frequency = o.patch("frequency", |v| v.enum_());
        let anchor_date = o.patch("anchor_date", |v| v.date());
        let is_active = o.patch("is_active", |v| v.bool());
        o.finish()?;
        Ok(Self {
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
            is_active: is_active.value(),
        })
    }
}

/// Path parameters of the `/api/recurring/{recurring_id}` routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecurringPath {
    pub recurring_id: i64,
}

impl FromInput for RecurringPath {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let recurring_id = o.required("recurring_id", |v| v.int());
        o.finish()?;
        Ok(Self { recurring_id: recurring_id.value() })
    }
}

// ---------------------------------------------------------------------------
// Response DTOs
// ---------------------------------------------------------------------------

/// `RecurringTransactionRead`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RecurringTransactionRead {
    pub id: i32,
    pub account_id: i32,
    pub account_name: String,
    pub category_id: Option<i32>,
    pub category_name: Option<String>,
    pub category_color: Option<String>,
    pub category_icon: Option<String>,
    pub transfer_account_id: Option<i32>,
    pub transfer_account_name: Option<String>,
    #[serde(rename = "type")]
    pub type_: TransactionType,
    pub amount: Decimal,
    pub destination_amount: Option<Decimal>,
    pub description: String,
    pub merchant: Option<String>,
    pub notes: Option<String>,
    pub frequency: RecurringFrequency,
    pub anchor_date: NaiveDate,
    pub last_posted_date: Option<NaiveDate>,
    pub is_active: bool,
    // Computed, not stored — see next_due_date below.
    pub next_due_date: NaiveDate,
    pub is_due: bool,
    pub days_until_due: i64,
}

// ---------------------------------------------------------------------------
// Pure logic
// ---------------------------------------------------------------------------

/// One step of the schedule. Monthly clamps to the target month's last day
/// (Jan 31 → Feb 28); yearly Feb 29 anchors fall back to Feb 28 in non-leap
/// years. `None` past year 9999 (Python raises there: a 500).
pub fn advance(day: NaiveDate, frequency: RecurringFrequency) -> Option<NaiveDate> {
    match frequency {
        RecurringFrequency::Weekly => day.checked_add_days(Days::new(7)).filter(|d| d.year() <= 9999),
        RecurringFrequency::Monthly => {
            let (year, month) = if day.month() == 12 { (day.year() + 1, 1) } else { (day.year(), day.month() + 1) };
            if year > 9999 {
                return None;
            }
            let last_day = NaiveDate::from_ymd_opt(year, month, 1)?.checked_add_months(chrono::Months::new(1))?;
            let last_day = last_day.pred_opt()?.day();
            NaiveDate::from_ymd_opt(year, month, day.day().min(last_day))
        }
        RecurringFrequency::Yearly => {
            let year = day.year() + 1;
            if year > 9999 {
                return None;
            }
            NaiveDate::from_ymd_opt(year, day.month(), day.day())
                .or_else(|| NaiveDate::from_ymd_opt(year, day.month(), 28))
        }
    }
}

/// First due date if never posted, otherwise one step after the last post —
/// so the schedule only advances when the user actually clicks Post.
pub fn next_due_date(
    anchor_date: NaiveDate,
    last_posted_date: Option<NaiveDate>,
    frequency: RecurringFrequency,
) -> Option<NaiveDate> {
    match last_posted_date {
        None => Some(anchor_date),
        Some(last) => advance(last, frequency),
    }
}

/// The second half of `_ensure_category_matches_type` (the "Category not
/// found" half needs IO): an income/expense template's category must have
/// the matching kind; transfers accept any.
pub fn category_kind_violation(category: &CategoryRead, type_: Option<TransactionType>) -> Option<String> {
    let expected = category_kind_for(type_)?;
    let type_ = type_?;
    (category.kind != expected).then(|| {
        format!(
            "Category '{}' is a {} category and cannot be used for a {} recurring transaction",
            category.name,
            category.kind.as_str(),
            type_.as_str()
        )
    })
}

/// What [`normalise_destination_amount`] needs looked up first: the two
/// accounts' currencies, or nothing (`None`) when it decides without them.
pub fn destination_account_ids(
    type_: Option<TransactionType>,
    account_id: Option<i64>,
    transfer_account_id: Option<i64>,
) -> Option<(Option<i64>, i64)> {
    if type_ != Some(TransactionType::Transfer) {
        return None;
    }
    let transfer_account_id = transfer_account_id?;
    (Some(transfer_account_id) != account_id).then_some((account_id, transfer_account_id))
}

/// `_normalise_destination_amount`: same-currency transfers always move the
/// same amount, cross-currency ones must say what reached the destination;
/// non-transfers carry no transfer fields at all. `currencies` maps account
/// id → currency for the accounts that exist (only consulted when
/// [`destination_account_ids`] asked for them). `Err` is the 400 detail.
pub fn normalise_destination_amount(
    type_: Option<TransactionType>,
    account_id: Option<i64>,
    transfer_account_id: Option<i64>,
    amount: Option<&Decimal>,
    destination_amount: Option<&Decimal>,
    currencies: &HashMap<i64, String>,
) -> Result<Option<Decimal>, &'static str> {
    if type_ != Some(TransactionType::Transfer) {
        if transfer_account_id.is_some() || destination_amount.is_some() {
            return Err("Transfer fields are only valid for transfers");
        }
        return Ok(None);
    }
    let Some((account_id, transfer_account_id)) = destination_account_ids(type_, account_id, transfer_account_id)
    else {
        return Err("A distinct destination account is required");
    };
    let source = account_id.and_then(|id| currencies.get(&id));
    let (Some(source), Some(destination)) = (source, currencies.get(&transfer_account_id)) else {
        return Err("Transfer account not found");
    };
    if source == destination {
        if destination_amount.is_some() && destination_amount != amount {
            return Err("Same-currency transfer amounts must match");
        }
        return Ok(amount.cloned());
    }
    match destination_amount {
        None => Err("destination_amount is required for a cross-currency transfer"),
        Some(d) => Ok(Some(d.clone())),
    }
}

/// A template's stored (or about-to-be-answered) columns plus the names its
/// relations resolved to — what `_to_read` reads off the ORM object.
#[derive(Debug, Clone, PartialEq)]
pub struct RecurringState {
    pub id: i32,
    pub account_id: i32,
    pub account_name: String,
    pub category_id: Option<i32>,
    pub category: Option<(String, String, Option<String>)>,
    pub transfer_account_id: Option<i32>,
    pub transfer_account_name: Option<String>,
    pub type_: TransactionType,
    pub amount: Decimal,
    pub destination_amount: Option<Decimal>,
    pub description: String,
    pub merchant: Option<String>,
    pub notes: Option<String>,
    pub frequency: RecurringFrequency,
    pub anchor_date: NaiveDate,
    pub last_posted_date: Option<NaiveDate>,
    pub is_active: bool,
}

/// `_to_read`: the template plus its computed schedule as of `today`.
/// `None` when the next due date is past year 9999 (a 500 under Python).
pub fn recurring_read(state: RecurringState, today: NaiveDate) -> Option<RecurringTransactionRead> {
    let next_due = next_due_date(state.anchor_date, state.last_posted_date, state.frequency)?;
    let (category_name, category_color, category_icon) = match state.category {
        Some((name, color, icon)) => (Some(name), Some(color), icon),
        None => (None, None, None),
    };
    Some(RecurringTransactionRead {
        id: state.id,
        account_id: state.account_id,
        account_name: state.account_name,
        category_id: state.category_id,
        category_name,
        category_color,
        category_icon,
        transfer_account_id: state.transfer_account_id,
        transfer_account_name: state.transfer_account_name,
        type_: state.type_,
        amount: state.amount,
        destination_amount: state.destination_amount,
        description: state.description,
        merchant: state.merchant,
        notes: state.notes,
        frequency: state.frequency,
        anchor_date: state.anchor_date,
        last_posted_date: state.last_posted_date,
        is_active: state.is_active,
        next_due_date: next_due,
        is_due: state.is_active && next_due <= today,
        days_until_due: (next_due - today).num_days(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    #[test]
    fn schedule_steps() {
        use RecurringFrequency::*;
        assert_eq!(advance(ymd(2026, 1, 31), Monthly), Some(ymd(2026, 2, 28)));
        assert_eq!(advance(ymd(2024, 1, 31), Monthly), Some(ymd(2024, 2, 29)));
        assert_eq!(advance(ymd(2026, 12, 15), Monthly), Some(ymd(2027, 1, 15)));
        assert_eq!(advance(ymd(2024, 2, 29), Yearly), Some(ymd(2025, 2, 28)));
        assert_eq!(advance(ymd(2026, 12, 29), Weekly), Some(ymd(2027, 1, 5)));
        assert_eq!(advance(ymd(9999, 12, 31), Weekly), None);
        assert_eq!(advance(ymd(9999, 12, 1), Monthly), None);
        assert_eq!(advance(ymd(9999, 1, 1), Yearly), None);
        assert_eq!(next_due_date(ymd(2026, 1, 15), None, Monthly), Some(ymd(2026, 1, 15)));
    }

    #[test]
    fn destination_rules() {
        let currencies: HashMap<i64, String> = [(1, "USD".to_string()), (2, "USD".into()), (3, "PLN".into())].into();
        let t = Some(TransactionType::Transfer);
        let e = Some(TransactionType::Expense);
        assert_eq!(
            normalise_destination_amount(e, Some(1), Some(2), Some(&d("5")), None, &currencies),
            Err("Transfer fields are only valid for transfers")
        );
        assert_eq!(normalise_destination_amount(e, Some(1), None, Some(&d("5")), None, &currencies), Ok(None));
        assert_eq!(
            normalise_destination_amount(t, Some(1), None, Some(&d("5")), None, &currencies),
            Err("A distinct destination account is required")
        );
        assert_eq!(
            normalise_destination_amount(t, Some(1), Some(1), Some(&d("5")), None, &currencies),
            Err("A distinct destination account is required")
        );
        assert_eq!(
            normalise_destination_amount(t, Some(1), Some(2), Some(&d("5.0")), None, &currencies),
            Ok(Some(d("5.0")))
        );
        assert_eq!(
            normalise_destination_amount(t, Some(1), Some(3), Some(&d("5")), None, &currencies),
            Err("destination_amount is required for a cross-currency transfer")
        );
        assert_eq!(
            normalise_destination_amount(t, Some(1), Some(9), Some(&d("5")), None, &currencies),
            Err("Transfer account not found")
        );
    }
}
