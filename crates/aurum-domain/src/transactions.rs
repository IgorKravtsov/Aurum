//! API contract types for `/api/transactions` — port of `backend/app/schemas/transaction.py`,
//! plus the pure write-time rules of `backend/app/api/routes/transactions.py`.
//!
//! Follows `crates/aurum-server/PORTING.md`: response DTOs derive
//! `Serialize` (field order = pydantic field order, inherited fields first),
//! request DTOs implement `validation::FromInput`.
//!
//! The rules take `Option`s where Python's PATCH path can hand them `None`
//! (a field explicitly patched to `null` is only rejected by the database
//! at commit time, after these checks ran on the `None`).

use std::collections::{BTreeSet, HashMap};

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::accounts::AccountRead;
use crate::categories::CategoryRead;
use crate::decimal::Decimal;
use crate::enums::{CategoryKind, TransactionType};
use crate::tags::TagRead;
use crate::text::capitalize_first_letter;
use crate::validation::{ErrorItem, FromInput, Patch, Res, Val, ValidationErrors};

/// `TransactionBulkCreate.items: list[...] = Field(min_length=1, max_length=5000)`.
pub const BULK_MAX_ITEMS: usize = 5000;

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

/// The transfer invariants in one place, returning the problem as text or
/// `None` if the combination is valid.
///
/// Both halves of the write path go through this: the create schema (as a
/// model validator) and the PATCH route. Enforcing it on create only used to
/// let an edit produce a row that create would have rejected — most
/// damagingly a transfer with no transfer_account_id, which TransactionRead
/// itself can't serialize, so the row broke every later read of the
/// transactions list. Ids are tested for truthiness like Python (`0` is
/// "missing").
pub fn transfer_rule_violation(
    type_: Option<TransactionType>,
    account_id: Option<i64>,
    transfer_account_id: Option<i64>,
    category_id: Option<i64>,
    destination_amount: Option<&Decimal>,
) -> Option<&'static str> {
    let truthy = |id: Option<i64>| matches!(id, Some(v) if v != 0);
    if type_ == Some(TransactionType::Transfer) {
        if !truthy(transfer_account_id) {
            return Some("transfer_account_id is required for transfer transactions");
        }
        if transfer_account_id == account_id {
            return Some("transfer_account_id must differ from account_id");
        }
        if truthy(category_id) {
            return Some("category_id is not valid for transfer transactions");
        }
    } else if truthy(transfer_account_id) {
        return Some("transfer_account_id is only valid for transfer transactions");
    } else if destination_amount.is_some() {
        return Some("destination_amount is only valid for transfer transactions");
    }
    None
}

/// The split invariants in one place, same shape as
/// [`transfer_rule_violation`] — checked on create, and against the row *as
/// it would look after a patch* on update, so an edit can't leave a
/// transaction whose splits no longer add up to its own amount.
///
/// Takes counts/totals rather than the split objects themselves: the update
/// path's "splits weren't touched by this patch" case has to check the
/// existing rows, whose category_id can be NULL (the category was since
/// deleted) — nothing here needs a live category to check the sum.
pub fn split_rule_violation(
    type_: Option<TransactionType>,
    amount: Option<&Decimal>,
    category_id: Option<i64>,
    split_count: usize,
    split_total: Option<&Decimal>,
) -> Option<String> {
    if split_count == 0 {
        return None;
    }
    if type_ == Some(TransactionType::Transfer) {
        return Some("splits are not valid for transfer transactions".into());
    }
    if category_id.is_some() {
        return Some("category_id must be omitted when splitting a transaction across categories".into());
    }
    if split_count < 2 {
        return Some("splitting a transaction needs at least 2 categories".into());
    }
    if split_total != amount {
        return Some(format!(
            "split amounts ({}) must add up to the transaction amount ({})",
            py_str_or_none(split_total),
            py_str_or_none(amount)
        ));
    }
    None
}

fn py_str_or_none(value: Option<&Decimal>) -> String {
    value.map_or_else(|| "None".to_string(), Decimal::to_string)
}

/// `sum((s.amount for s in splits), Decimal("0")) if splits else None`.
pub fn split_total<'a>(amounts: impl IntoIterator<Item = &'a Decimal>) -> Option<Decimal> {
    let mut amounts = amounts.into_iter().peekable();
    amounts.peek()?;
    Some(amounts.fold(Decimal::zero(), |sum, amount| sum.add(amount)))
}

/// `_TYPE_TO_CATEGORY_KIND`: transfers (and a `None` type) have no kind.
pub fn category_kind_for(type_: Option<TransactionType>) -> Option<CategoryKind> {
    match type_ {
        Some(TransactionType::Income) => Some(CategoryKind::Income),
        Some(TransactionType::Expense) => Some(CategoryKind::Expense),
        _ => None,
    }
}

/// A category picked for an income transaction must itself be an income
/// category (and likewise for expense) — otherwise the dashboard's spending
/// breakdown, which only joins EXPENSE-typed rows, would silently
/// misclassify the entry. Port of the second half of
/// `_ensure_category_matches_type` (the "Category not found" half needs IO).
pub fn category_kind_violation(category: &CategoryRead, type_: Option<TransactionType>) -> Option<String> {
    let expected = category_kind_for(type_)?;
    let type_ = type_?;
    (category.kind != expected).then(|| {
        format!(
            "Category '{}' is a {} category and cannot be used for a {} transaction",
            category.name,
            category.kind.as_str(),
            type_.as_str()
        )
    })
}

/// A split's whole point is dividing one purchase's total across the
/// *subcategories of one parent* — each split may point at that parent
/// itself or at one of its direct children, enforced by requiring every
/// split's own top-level ancestor (parent_id, or its own id) to agree.
pub fn split_parent_violation<'a>(categories: impl IntoIterator<Item = &'a CategoryRead>) -> Option<&'static str> {
    let top_level: BTreeSet<i32> = categories.into_iter().map(|c| c.parent_id.unwrap_or(c.id)).collect();
    (top_level.len() > 1).then_some("All split categories must be the same parent category or its direct subcategories")
}

/// Whether [`normalise_destination_amount`] needs the two accounts'
/// currencies (Python only queries them in that case).
pub fn destination_needs_currencies(type_: Option<TransactionType>, transfer_account_id: Option<i64>) -> bool {
    type_ == Some(TransactionType::Transfer) && transfer_account_id.is_some()
}

/// Makes transfer legs explicit: same-currency transfers always move the
/// same amount, while cross-currency transfers must record what actually
/// reached the destination account. `currencies` maps account id →
/// currency for the accounts that exist (only consulted when
/// [`destination_needs_currencies`]). `Err` is the 400 detail.
pub fn normalise_destination_amount(
    type_: Option<TransactionType>,
    account_id: Option<i64>,
    transfer_account_id: Option<i64>,
    amount: Option<&Decimal>,
    destination_amount: Option<&Decimal>,
    currencies: &HashMap<i64, String>,
) -> Result<Option<Decimal>, &'static str> {
    if type_ != Some(TransactionType::Transfer) {
        return Ok(None);
    }
    let Some(transfer_account_id) = transfer_account_id else {
        return Ok(destination_amount.cloned());
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

/// `f"Unknown tag id(s): {sorted(missing)}"`, or `None` when every
/// requested id was found.
pub fn unknown_tags_detail(requested: &[i64], found: &[TagRead]) -> Option<String> {
    let missing: BTreeSet<i64> =
        requested.iter().copied().filter(|id| !found.iter().any(|t| i64::from(t.id) == *id)).collect();
    if missing.is_empty() {
        return None;
    }
    let list: Vec<String> = missing.iter().map(i64::to_string).collect();
    Some(format!("Unknown tag id(s): [{}]", list.join(", ")))
}

/// Full range of years to offer in the year picker — from the earliest
/// transaction through the current year, so a gap year with no activity
/// still shows up (as zero) instead of silently disappearing from the UI.
pub fn year_range(bounds: Option<(NaiveDate, NaiveDate)>, current_year: i32) -> Vec<i32> {
    use chrono::Datelike;
    match bounds {
        None => vec![current_year],
        Some((min, max)) => (min.year()..=max.year().max(current_year)).collect(),
    }
}

// ---------------------------------------------------------------------------
// Request DTOs
// ---------------------------------------------------------------------------

fn amount_field<'a>(v: Val<'a>) -> Res<'a, Decimal> {
    // Field(gt=0, max_digits=14, decimal_places=2) — digits before bounds.
    v.decimal()?.digits(Some(14), Some(2))?.gt("0")
}

fn description_field<'a>(v: Val<'a>) -> Res<'a, String> {
    // Auto-capitalizes "траты на продукты" -> "Траты на продукты" so mixed
    // casing from quick manual entry doesn't need fixing by hand later.
    Ok(v.str()?.min_len(1)?.max_len(255)?.map(|s| capitalize_first_letter(&s)))
}

/// `TransactionSplitInput` — one category's slice of a split transaction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TransactionSplitInput {
    pub category_id: i64,
    pub amount: Decimal,
    pub note: Option<String>,
}

impl FromInput for TransactionSplitInput {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let category_id = o.required("category_id", |v| v.int());
        let amount = o.required("amount", amount_field);
        let note = o.optional("note", |v| v.str()?.max_len(200));
        o.finish()?;
        Ok(Self { category_id: category_id.value(), amount: amount.value(), note: note.value() })
    }
}

/// `TransactionCreate` — body of `POST /api/transactions` and one item of
/// the bulk import.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TransactionCreate {
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
    pub date: NaiveDate,
    pub tag_ids: Vec<i64>,
    // None/omitted -> a normal single-category transaction. 2+ entries ->
    // the amount is divided across categories instead of using category_id
    // (which must then be omitted — see split_rule_violation). A single
    // entry isn't accepted: that's just category_id with extra steps.
    pub splits: Option<Vec<TransactionSplitInput>>,
}

impl FromInput for TransactionCreate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let account_id = o.required("account_id", |v| v.int());
        let category_id = o.optional("category_id", |v| v.int());
        let transfer_account_id = o.optional("transfer_account_id", |v| v.int());
        let type_ = o.required("type", |v| v.enum_());
        let amount = o.required("amount", amount_field);
        let destination_amount = o.optional("destination_amount", amount_field);
        let description = o.required("description", description_field);
        let merchant = o.optional("merchant", |v| v.str()?.max_len(150));
        let notes = o.optional("notes", |v| v.str()?.max_len(2000));
        let date = o.required("date", |v| v.date());
        let tag_ids = o.with_default("tag_ids", Vec::new, |v| v.list(|v| v.int()));
        let splits = o.optional("splits", |v| v.list(|v| v.model::<TransactionSplitInput>()));
        o.finish()?;
        let this = Self {
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
            splits: splits.value(),
        };
        // TransactionBase's model validator runs before TransactionCreate's.
        if let Some(violation) = transfer_rule_violation(
            Some(this.type_),
            Some(this.account_id),
            this.transfer_account_id,
            this.category_id,
            this.destination_amount.as_ref(),
        ) {
            return Err(v.model_value_error(violation));
        }
        let splits = this.splits.as_deref().unwrap_or_default();
        let total = split_total(splits.iter().map(|s| &s.amount));
        if let Some(violation) =
            split_rule_violation(Some(this.type_), Some(&this.amount), this.category_id, splits.len(), total.as_ref())
        {
            return Err(v.model_value_error(&violation));
        }
        Ok(this)
    }
}

/// `TransactionUpdate` — partial update (`exclude_unset`).
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TransactionUpdate {
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
    #[cfg_attr(feature = "openapi", schema(value_type = Option<NaiveDate>))]
    pub date: Patch<NaiveDate>,
    // Omitted -> tags untouched; sent (even as []) -> replaces the full tag set.
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Vec<i64>>))]
    pub tag_ids: Patch<Vec<i64>>,
    // Omitted -> splits untouched; sent (even as []) -> replaces the full
    // split set (send [] together with a category_id to turn a split
    // transaction back into a normal single-category one).
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Vec<TransactionSplitInput>>))]
    pub splits: Patch<Vec<TransactionSplitInput>>,
}

impl FromInput for TransactionUpdate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let account_id = o.patch("account_id", |v| v.int());
        let category_id = o.patch("category_id", |v| v.int());
        let transfer_account_id = o.patch("transfer_account_id", |v| v.int());
        let type_ = o.patch("type", |v| v.enum_());
        let amount = o.patch("amount", amount_field);
        let destination_amount = o.patch("destination_amount", amount_field);
        let description = o.patch("description", description_field);
        let merchant = o.patch("merchant", |v| v.str()?.max_len(150));
        let notes = o.patch("notes", |v| v.str()?.max_len(2000));
        let date = o.patch("date", |v| v.date());
        let tag_ids = o.patch("tag_ids", |v| v.list(|v| v.int()));
        let splits = o.patch("splits", |v| v.list(|v| v.model::<TransactionSplitInput>()));
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
            date: date.value(),
            tag_ids: tag_ids.value(),
            splits: splits.value(),
        })
    }
}

/// `TransactionBulkCreate` — CSV import: the frontend parses the file and
/// maps its columns client-side, then sends already-shaped rows here.
/// All-or-nothing — same failure semantics as backup restore, so a single
/// bad row never leaves a partial import behind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TransactionBulkCreate {
    pub items: Vec<TransactionCreate>,
}

impl FromInput for TransactionBulkCreate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let items = o.required("items", |v| {
            // pydantic-core rejects an over-long list before validating any item.
            if let Value::Array(raw) = v.value
                && raw.len() > BULK_MAX_ITEMS
            {
                return Err(list_length_error(&v, "too_long", raw.len()).into());
            }
            let items = v.list(|v| v.model::<TransactionCreate>())?;
            if items.value.is_empty() {
                return Err(list_length_error(&v, "too_short", 0).into());
            }
            Ok(items)
        });
        o.finish()?;
        Ok(Self { items: items.value() })
    }
}

/// pydantic's `too_short` / `too_long` for `items`.
fn list_length_error(v: &Val<'_>, kind: &str, actual: usize) -> ErrorItem {
    let (msg, bound) = if kind == "too_short" {
        (format!("List should have at least 1 item after validation, not {actual}"), json!({ "min_length": 1 }))
    } else {
        (
            format!("List should have at most {BULK_MAX_ITEMS} items after validation, not {actual}"),
            json!({ "max_length": BULK_MAX_ITEMS }),
        )
    };
    let mut ctx = json!({ "field_type": "List" });
    if let (Value::Object(ctx), Value::Object(bound)) = (&mut ctx, bound) {
        ctx.extend(bound);
        ctx.insert("actual_length".into(), json!(actual));
    }
    ErrorItem::new(kind, v.loc.clone(), msg, v.value.clone()).with_ctx(ctx)
}

/// `GET /api/transactions` sort order (`Literal[...]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransactionSort {
    #[default]
    DateDesc,
    AmountDesc,
    AmountAsc,
}

impl TransactionSort {
    pub const VALUES: &'static [&'static str] = &["date_desc", "amount_desc", "amount_asc"];

    pub fn as_str(self) -> &'static str {
        match self {
            TransactionSort::DateDesc => "date_desc",
            TransactionSort::AmountDesc => "amount_desc",
            TransactionSort::AmountAsc => "amount_asc",
        }
    }

    fn from_literal(value: &str) -> Self {
        match value {
            "amount_desc" => TransactionSort::AmountDesc,
            "amount_asc" => TransactionSort::AmountAsc,
            _ => TransactionSort::DateDesc,
        }
    }
}

/// Query of `GET /api/transactions` (FastAPI lists errors in signature order).
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionListQuery {
    pub year: Option<i64>,
    pub month: Option<i64>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub account_id: Option<i64>,
    pub category_id: Option<i64>,
    pub tag_id: Option<i64>,
    pub type_: Option<TransactionType>,
    pub search: Option<String>,
    pub sort: TransactionSort,
    pub page: i64,
    pub page_size: i64,
}

impl FromInput for TransactionListQuery {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let year = o.optional("year", |v| v.int()?.ge(2000)?.le(2100));
        let month = o.optional("month", |v| v.int()?.ge(1)?.le(12));
        let start_date = o.optional("start_date", |v| v.date());
        let end_date = o.optional("end_date", |v| v.date());
        let account_id = o.optional("account_id", |v| v.int());
        let category_id = o.optional("category_id", |v| v.int());
        let tag_id = o.optional("tag_id", |v| v.int());
        let type_ = o.optional("type", |v| v.enum_());
        let search = o.optional("search", |v| v.str()?.min_len(1)?.max_len(255));
        let sort = o.with_default("sort", TransactionSort::default, |v| {
            Ok(v.literal(TransactionSort::VALUES)?.map(TransactionSort::from_literal))
        });
        let page = o.with_default("page", || 1, |v| v.int()?.ge(1));
        let page_size = o.with_default("page_size", || 20, |v| v.int()?.ge(1)?.le(200));
        o.finish()?;
        Ok(Self {
            year: year.value(),
            month: month.value(),
            start_date: start_date.value(),
            end_date: end_date.value(),
            account_id: account_id.value(),
            category_id: category_id.value(),
            tag_id: tag_id.value(),
            type_: type_.value(),
            search: search.value(),
            sort: sort.value(),
            page: page.value(),
            page_size: page_size.value(),
        })
    }
}

/// Path parameters of `PATCH/DELETE /api/transactions/{transaction_id}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionPath {
    pub transaction_id: i64,
}

impl FromInput for TransactionPath {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let transaction_id = o.required("transaction_id", |v| v.int());
        o.finish()?;
        Ok(Self { transaction_id: transaction_id.value() })
    }
}

// ---------------------------------------------------------------------------
// Response DTOs
// ---------------------------------------------------------------------------

/// `TransactionSplitRead`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TransactionSplitRead {
    pub id: i32,
    pub category_id: Option<i32>,
    pub category: Option<CategoryRead>,
    pub amount: Decimal,
    pub note: Option<String>,
}

/// `TransactionRead` — `TransactionFields` (read without the write-time
/// rules: rows already in the database can stop satisfying them, e.g. a
/// transfer whose destination account was deleted) plus the relations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TransactionRead {
    pub account_id: i32,
    pub category_id: Option<i32>,
    pub transfer_account_id: Option<i32>,
    #[serde(rename = "type")]
    pub type_: TransactionType,
    pub amount: Decimal,
    pub destination_amount: Option<Decimal>,
    pub description: String,
    pub merchant: Option<String>,
    pub notes: Option<String>,
    pub date: NaiveDate,
    pub id: i32,
    pub account: AccountRead,
    pub category: Option<CategoryRead>,
    pub tags: Vec<TagRead>,
    pub splits: Vec<TransactionSplitRead>,
}

/// `TransactionPage`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TransactionPage {
    pub items: Vec<TransactionRead>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

/// `TransactionBulkCreateResult`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TransactionBulkCreateResult {
    pub created: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::Source;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    #[test]
    fn transfer_rules() {
        let t = Some(TransactionType::Transfer);
        assert_eq!(
            transfer_rule_violation(t, Some(1), Some(0), None, None),
            Some("transfer_account_id is required for transfer transactions")
        );
        assert_eq!(
            transfer_rule_violation(t, Some(1), Some(1), None, None),
            Some("transfer_account_id must differ from account_id")
        );
        assert_eq!(transfer_rule_violation(t, Some(1), Some(2), Some(0), None), None);
        assert_eq!(
            transfer_rule_violation(Some(TransactionType::Expense), Some(1), None, None, Some(&d("1"))),
            Some("destination_amount is only valid for transfer transactions")
        );
        assert_eq!(transfer_rule_violation(None, None, None, None, None), None);
    }

    #[test]
    fn split_rules_print_python_values() {
        let total = split_total([&d("3.5"), &d("3.50")]);
        assert_eq!(total.as_ref().map(ToString::to_string).as_deref(), Some("7.00"));
        assert_eq!(
            split_rule_violation(Some(TransactionType::Expense), None, None, 2, total.as_ref()).as_deref(),
            Some("split amounts (7.00) must add up to the transaction amount (None)")
        );
        assert_eq!(split_rule_violation(Some(TransactionType::Expense), Some(&d("7")), None, 2, total.as_ref()), None);
        assert_eq!(split_total(std::iter::empty()), None);
    }

    #[test]
    fn destination_amount_normalisation() {
        let currencies: HashMap<i64, String> = [(1, "USD".to_string()), (2, "USD".into()), (3, "PLN".into())].into();
        let t = Some(TransactionType::Transfer);
        assert_eq!(
            normalise_destination_amount(t, Some(1), Some(2), Some(&d("5.0")), None, &currencies).unwrap(),
            Some(d("5.0"))
        );
        assert_eq!(
            normalise_destination_amount(t, Some(1), Some(2), Some(&d("5")), Some(&d("6")), &currencies),
            Err("Same-currency transfer amounts must match")
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

    #[test]
    fn unknown_tags_and_years() {
        let found = vec![TagRead { id: 2, name: "x".into() }];
        assert_eq!(unknown_tags_detail(&[5, 2, 3, 5], &found).as_deref(), Some("Unknown tag id(s): [3, 5]"));
        assert_eq!(unknown_tags_detail(&[2], &found), None);
        let jan = |y| NaiveDate::from_ymd_opt(y, 1, 1).unwrap();
        assert_eq!(year_range(None, 2026), vec![2026]);
        assert_eq!(year_range(Some((jan(2024), jan(2025))), 2026), vec![2024, 2025, 2026]);
    }

    #[test]
    fn create_runs_parent_validator_first() {
        let body = json!({"account_id": 1, "type": "expense", "amount": "1", "description": "z",
            "date": "2026-01-16", "transfer_account_id": 5, "splits": [{"category_id": 1, "amount": "1"}]});
        let errors = TransactionCreate::from_source(&body, Source::Body).unwrap_err();
        assert_eq!(errors.0[0].msg, "Value error, transfer_account_id is only valid for transfer transactions");
    }

    #[test]
    fn bulk_length_errors() {
        let errors = TransactionBulkCreate::from_source(&json!({"items": []}), Source::Body).unwrap_err();
        assert_eq!(
            serde_json::to_string(&errors.0[0]).unwrap(),
            r#"{"type":"too_short","loc":["body","items"],"msg":"List should have at least 1 item after validation, not 0","input":[],"ctx":{"field_type":"List","min_length":1,"actual_length":0}}"#
        );
    }
}
