//! API contract types for `/api/net-worth` — port of `backend/app/schemas/net_worth.py`,
//! plus the pure aggregation of `backend/app/services/net_worth_service.py`.
//!
//! Net worth is a single trend line plus a percent breakdown by asset class.
//!
//! Design note: "Cash" is not stored anywhere — it's derived live from
//! Account/Transaction data (checking/debit-card/savings/cash/investment
//! accounts) so the Transactions feature stays the single source of truth
//! for liquid money. Investment accounts count here too: their balance is
//! uninvested/unallocated money sitting on the account, not the market value
//! of what's actually invested — that value is still tracked manually via
//! Asset/AssetValuation, same as every other class (investments, crypto,
//! real estate, vehicles, precious metals, other). Both halves are collapsed
//! into one daily, forward-filled series so the chart reads as one
//! continuous line even though the two halves are updated at very different
//! cadences.
//!
//! The service is split into IO-free phases so mobile clients can reuse it:
//! [`cash_plan`] / [`asset_plan`] say which native amounts must be converted
//! into the display currency (the server does that via NBP), then
//! [`CashPlan::events`] / [`AssetPlan::totals`], [`capital_role_summary`],
//! [`risk_level_summary`] and [`assemble_summary`] build the response.
//!
//! Parity: Python sums converted (28-significant-digit) Decimals, so the
//! summation *order* can change the last digit. Every sum here follows the
//! order Python's dicts had (row order of the same SQL, insertion order).

use std::collections::{HashMap, HashSet};

use chrono::{Duration, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::decimal::Decimal;
use crate::enums::{AccountType, AssetClass, CapitalRole, RiskLevel, TransactionType};
use crate::fx::MoneyAmount;
use crate::pyfmt::PyFloat;
use crate::validation::{FromInput, Val, ValidationErrors};

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

/// `NetWorthPoint`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NetWorthPoint {
    pub date: NaiveDate,
    pub value: Decimal,
}

/// `NetWorthBreakdownItem`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NetWorthBreakdownItem {
    pub key: String,
    pub name: String,
    pub color: String,
    pub icon: String,
    pub amount: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = f64))]
    pub percent: PyFloat,
}

/// Assets grouped by how they behave month to month (see CapitalRole) — a
/// cross-cut of the asset-class breakdown, not a replacement for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CapitalRoleSummary {
    pub role: String,
    pub label: String,
    pub color: String,
    pub total_value: Decimal,
    pub monthly_cash_flow: Decimal,
    pub count: i64,
}

/// One holding within a risk tier — Cash (key="cash") or a single asset
/// (key="asset:{id}"). This list IS the diversification view: a tier with
/// one item at 100% is concentrated, several items with even shares aren't,
/// no separate score needed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RiskLevelItem {
    pub key: String,
    pub name: String,
    pub amount: Decimal,
    /// share of this tier, not of total capital
    #[cfg_attr(feature = "openapi", schema(value_type = f64))]
    pub percent: PyFloat,
}

/// Cash + assets grouped by user-tagged risk of loss (see RiskLevel) —
/// another cross-cut, like capital_roles, but Cash participates here since
/// it's the zero-risk anchor an 80/20-style allocation rule needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RiskLevelSummary {
    pub risk_level: String,
    pub label: String,
    pub color: String,
    pub total_value: Decimal,
    /// share of total capital (cash + assets)
    #[cfg_attr(feature = "openapi", schema(value_type = f64))]
    pub percent: PyFloat,
    pub items: Vec<RiskLevelItem>,
}

/// `NetWorthSummary` — response of `GET /api/net-worth/summary`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NetWorthSummary {
    pub range: String,
    pub current: Decimal,
    pub change_amount: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<f64>))]
    pub change_percent: Option<PyFloat>,
    pub series: Vec<NetWorthPoint>,
    pub breakdown: Vec<NetWorthBreakdownItem>,
    pub capital_roles: Vec<CapitalRoleSummary>,
    pub risk_levels: Vec<RiskLevelSummary>,
}

/// `RANGE_DAYS`: how far back each fixed range reaches (`"all"` reaches the
/// first transaction/valuation instead).
pub const RANGE_DAYS: &[(&str, i64)] = &[("30d", 30), ("90d", 90), ("1y", 365), ("5y", 365 * 5)];

/// The route's `_RANGE_PATTERN`: `sorted(set(RANGE_DAYS) | {"all"})` joined.
pub const RANGE_PATTERN: &str = r"^(1y|30d|5y|90d|all)$";

/// Query of `GET /api/net-worth/summary`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetWorthQuery {
    pub range: String,
}

impl FromInput for NetWorthQuery {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let range = o.with_default("range", || "30d".to_string(), |v| v.str()?.pattern(RANGE_PATTERN));
        o.finish()?;
        Ok(Self { range: range.value() })
    }
}

/// Days covered by a fixed range key, `None` for `"all"`.
pub fn range_days(range_key: &str) -> Option<i64> {
    RANGE_DAYS.iter().find(|(key, _)| *key == range_key).map(|(_, days)| *days)
}

/// First chart day of a fixed range (`today - (days - 1)`), `None` for `"all"`.
pub fn requested_start(range_key: &str, today: NaiveDate) -> Option<NaiveDate> {
    range_days(range_key).map(|days| today - Duration::days(days - 1))
}

// ---------------------------------------------------------------------------
// Display metadata
// ---------------------------------------------------------------------------

/// Account types whose balance counts as Cash.
pub const CASH_ACCOUNT_TYPES: &[AccountType] =
    &[AccountType::Checking, AccountType::DebitCard, AccountType::Savings, AccountType::Cash, AccountType::Investment];

/// Breakdown key of the derived Cash slice.
pub const CASH_KEY: &str = "cash";

/// `_CLASS_META`: key -> (display name, color, lucide icon). Colors follow
/// the dataviz skill's validated 8-slot order (blue, orange, aqua, yellow,
/// magenta, green, violet, red) in this exact sequence — an
/// adjacent-pair-safe order for a segmented bar/donut; do not reorder
/// without re-validating.
pub fn class_meta(key: &str) -> (&'static str, &'static str, &'static str) {
    match key {
        "cash" => ("Наличные", "#2a78d6", "wallet"),                // slot 1 blue
        "investments" => ("Инвестиции", "#eb6834", "trending-up"),  // slot 2 orange
        "crypto" => ("Криптовалюта", "#1baf7a", "bitcoin"),         // slot 3 aqua
        "real_estate" => ("Недвижимость", "#eda100", "building-2"), // slot 4 yellow
        "vehicles" => ("Транспорт", "#e87ba4", "car"),              // slot 5 magenta
        "precious_metals" => ("Драгметаллы", "#008300", "gem"),     // slot 6 green
        _ => ("Прочее", "#4a3aa7", "package"),                      // slot 7 violet
    }
}

/// `_ROLE_META`: role -> (label, color). Status colors, not categorical
/// ones — "good" and "critical" from the dataviz skill's fixed status
/// palette, reused verbatim from how StatCard/NetWorthChart already color
/// income vs. expense elsewhere in the app.
pub fn role_meta(role: CapitalRole) -> (&'static str, &'static str) {
    match role {
        CapitalRole::Income => ("Доходный", "var(--success)"),
        CapitalRole::Neutral => ("Нейтральный", "var(--text-muted)"),
        CapitalRole::Drain => ("Убыточный", "var(--danger)"),
    }
}

/// `_RISK_META` — same status-color reuse as [`role_meta`]: low risk reads
/// as "good", high risk as "critical", not a categorical hue.
pub fn risk_meta(level: RiskLevel) -> (&'static str, &'static str) {
    match level {
        RiskLevel::Low => ("Низкий", "var(--success)"),
        RiskLevel::Medium => ("Средний", "var(--text-muted)"),
        RiskLevel::High => ("Высокий", "var(--danger)"),
    }
}

// ---------------------------------------------------------------------------
// Arithmetic helpers
// ---------------------------------------------------------------------------

/// `float(amount / denominator * 100) if denominator else 0.0`.
///
/// Python keeps the sign of a zero quotient (`0 / -50` is `Decimal("-0")`,
/// i.e. `-0.0` in JSON); our Decimal has no negative zero, so the sign is
/// restored here.
pub fn py_percent(amount: &Decimal, denominator: &Decimal) -> f64 {
    let Some(quotient) = amount.div(denominator) else {
        return 0.0;
    };
    let value = quotient.mul(&Decimal::from(100i64)).to_f64();
    if value == 0.0 && amount.is_negative() != denominator.is_negative() { -0.0 } else { value }
}

/// A Python `dict` with `defaultdict(Decimal)` semantics: insertion-ordered,
/// every value starting at `Decimal("0")`.
#[derive(Debug, Clone)]
struct OrderedSums<K> {
    keys: Vec<K>,
    sums: HashMap<K, Decimal>,
}

impl<K: Copy + Eq + std::hash::Hash> OrderedSums<K> {
    fn new() -> Self {
        Self { keys: Vec::new(), sums: HashMap::new() }
    }

    fn add(&mut self, key: K, amount: &Decimal) {
        let sum = self.sums.entry(key).or_insert_with(|| {
            self.keys.push(key);
            Decimal::zero()
        });
        *sum = sum.add(amount);
    }

    /// `d.get(key, Decimal("0"))`.
    fn get(&self, key: &K) -> Decimal {
        self.sums.get(key).cloned().unwrap_or_else(Decimal::zero)
    }

    /// `sum(d.values(), Decimal("0"))`.
    fn total(&self) -> Decimal {
        self.keys.iter().fold(Decimal::zero(), |acc, k| acc.add(&self.sums[k]))
    }
}

fn days(first: NaiveDate, last: NaiveDate) -> impl Iterator<Item = NaiveDate> {
    first.iter_days().take_while(move |day| *day <= last)
}

// ---------------------------------------------------------------------------
// Cash half
// ---------------------------------------------------------------------------

/// One `accounts` row (`select(Account.id, Account.type, Account.currency)`).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct CashAccountRow {
    pub id: i32,
    #[cfg_attr(feature = "sqlx", sqlx(rename = "type"))]
    pub type_: AccountType,
    pub currency: String,
}

/// One `transactions` row as far as the cash series is concerned.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct CashTransactionRow {
    pub date: NaiveDate,
    #[cfg_attr(feature = "sqlx", sqlx(rename = "type"))]
    pub type_: TransactionType,
    pub amount: Decimal,
    pub destination_amount: Option<Decimal>,
    pub account_id: i32,
    pub transfer_account_id: Option<i32>,
}

/// Every cash account's native balance on every chart day, to be converted
/// into the display currency (keys are `(day, account_id)`).
#[derive(Debug, Clone, PartialEq)]
pub struct CashPlan {
    pub first_day: NaiveDate,
    pub today: NaiveDate,
    /// Cash account ids in row order (the order Python summed them in).
    pub accounts: Vec<i32>,
    pub money: Vec<MoneyAmount<(NaiveDate, i32)>>,
}

/// First half of `_cash_cumulative_events`. `None` means "no events" (no
/// transactions at all, or a range starting after today).
///
/// Revalues the native balance of every account on every chart date: this
/// captures FX gains/losses after the transaction date instead of freezing
/// today's net worth at the historical purchase rate.
pub fn cash_plan(
    accounts: &[CashAccountRow],
    transactions: &[CashTransactionRow],
    series_start: Option<NaiveDate>,
    today: NaiveDate,
) -> Option<CashPlan> {
    let mut cash_accounts: Vec<(i32, &str)> = Vec::new();
    let mut seen = HashSet::new();
    for account in accounts {
        if CASH_ACCOUNT_TYPES.contains(&account.type_) && seen.insert(account.id) {
            cash_accounts.push((account.id, account.currency.as_str()));
        }
    }
    let is_cash = |id: Option<i32>| id.is_some_and(|id| seen.contains(&id));

    let mut delta_by_day_account: OrderedSums<(NaiveDate, i32)> = OrderedSums::new();
    for tx in transactions {
        let key = (tx.date, tx.account_id);
        match tx.type_ {
            TransactionType::Income if is_cash(Some(tx.account_id)) => delta_by_day_account.add(key, &tx.amount),
            TransactionType::Expense if is_cash(Some(tx.account_id)) => delta_by_day_account.add(key, &tx.amount.neg()),
            TransactionType::Transfer => {
                if is_cash(Some(tx.account_id)) {
                    delta_by_day_account.add(key, &tx.amount.neg());
                }
                if let Some(to) = tx.transfer_account_id.filter(|id| seen.contains(id)) {
                    // `destination_amount or amount`: a zero destination is falsy too.
                    let incoming = match &tx.destination_amount {
                        Some(d) if !d.is_zero() => d,
                        _ => &tx.amount,
                    };
                    delta_by_day_account.add((tx.date, to), incoming);
                }
            }
            _ => {}
        }
    }

    let history_start = transactions.iter().map(|tx| tx.date).min()?;
    let first_day = series_start.map_or(history_start, |start| start.max(history_start));
    if first_day > today {
        return None;
    }

    let mut native: HashMap<i32, Decimal> = HashMap::new();
    for key in &delta_by_day_account.keys {
        if key.0 < first_day {
            let balance = native.entry(key.1).or_insert_with(Decimal::zero);
            *balance = balance.add(&delta_by_day_account.sums[key]);
        }
    }
    let mut money = Vec::with_capacity(cash_accounts.len() * ((today - first_day).num_days() as usize + 1));
    for day in days(first_day, today) {
        for (account_id, currency) in &cash_accounts {
            let balance = native.entry(*account_id).or_insert_with(Decimal::zero);
            *balance = balance.add(&delta_by_day_account.get(&(day, *account_id)));
            money.push(MoneyAmount::new((day, *account_id), balance.clone(), *currency, day));
        }
    }
    Some(CashPlan { first_day, today, accounts: cash_accounts.iter().map(|(id, _)| *id).collect(), money })
}

impl CashPlan {
    /// Second half of `_cash_cumulative_events`: the converted cash total of
    /// every day (cumulative, so the last one *is* today's cash).
    pub fn events(&self, converted: &HashMap<(NaiveDate, i32), Decimal>) -> Vec<(NaiveDate, Decimal)> {
        days(self.first_day, self.today)
            .map(|day| {
                let total = self
                    .accounts
                    .iter()
                    .fold(Decimal::zero(), |acc, id| acc.add(converted.get(&(day, *id)).unwrap_or(&Decimal::zero())));
                (day, total)
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Asset half
// ---------------------------------------------------------------------------

/// One `assets` row (`select(Asset.id, Asset.asset_class, Asset.currency)`).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct AssetMetaRow {
    pub id: i32,
    pub asset_class: AssetClass,
    pub currency: String,
}

/// One `asset_valuations` row, in `as_of_date` order.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct ValuationRow {
    pub asset_id: i32,
    pub as_of_date: NaiveDate,
    pub value: Decimal,
}

/// Every asset's latest known native value on every chart day, to be
/// converted (keys are `(day, asset_id)`).
#[derive(Debug, Clone, PartialEq)]
pub struct AssetPlan {
    pub first_day: NaiveDate,
    pub today: NaiveDate,
    /// `asset_meta` in row order: (id, class).
    pub assets: Vec<(i32, AssetClass)>,
    /// Insertion order of Python's `native_by_asset` dict, which decides
    /// the summation order of the class totals.
    pub valued_assets: Vec<i32>,
    pub money: Vec<MoneyAmount<(NaiveDate, i32)>>,
}

/// What the asset half yields: the daily series, today's converted total
/// per class and per asset (both in Python's dict insertion order).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AssetTotals {
    pub events: Vec<(NaiveDate, Decimal)>,
    pub class_totals: Vec<(AssetClass, Decimal)>,
    pub current_by_asset: Vec<(i32, Decimal)>,
}

impl AssetTotals {
    /// `current_by_asset.get(asset_id, Decimal("0"))`.
    pub fn current_value(&self, asset_id: i32) -> Decimal {
        self.current_by_asset.iter().find(|(id, _)| *id == asset_id).map_or_else(Decimal::zero, |(_, v)| v.clone())
    }

    /// `class_totals.get(asset_class, Decimal("0"))`.
    pub fn class_total(&self, asset_class: AssetClass) -> Decimal {
        self.class_totals.iter().find(|(c, _)| *c == asset_class).map_or_else(Decimal::zero, |(_, v)| v.clone())
    }

    /// `sum(class_totals.values(), Decimal("0"))`.
    pub fn classes_sum(&self) -> Decimal {
        self.class_totals.iter().fold(Decimal::zero(), |acc, (_, v)| acc.add(v))
    }
}

/// First half of `_asset_events_and_class_totals`. `None` means "no
/// events" (no valuations, or every one after today / the range).
///
/// Valuations remain native to each asset. Revaluing the latest known
/// native amount on every date keeps historical and current net worth
/// sensitive to FX movements even between valuation entries.
pub fn asset_plan(
    assets: &[AssetMetaRow],
    valuations: &[ValuationRow],
    series_start: Option<NaiveDate>,
    today: NaiveDate,
) -> Option<AssetPlan> {
    let history_start = valuations.iter().map(|row| row.as_of_date).min()?;
    let first_day = series_start.map_or(history_start, |start| start.max(history_start));
    if first_day > today {
        return None;
    }
    let values_on_day: HashMap<(NaiveDate, i32), &Decimal> =
        valuations.iter().map(|row| ((row.as_of_date, row.asset_id), &row.value)).collect();

    // asset_meta = {id: (class, currency)}: a dict, so the last duplicate
    // wins but keeps the first position (ids are unique anyway).
    let mut meta: Vec<(i32, AssetClass, &str)> = Vec::new();
    for asset in assets {
        match meta.iter_mut().find(|(id, _, _)| *id == asset.id) {
            Some(entry) => *entry = (asset.id, asset.asset_class, asset.currency.as_str()),
            None => meta.push((asset.id, asset.asset_class, asset.currency.as_str())),
        }
    }

    let mut native: HashMap<i32, Decimal> = HashMap::new();
    let mut valued_assets: Vec<i32> = Vec::new();
    let mut set_native = |native: &mut HashMap<i32, Decimal>, id: i32, value: &Decimal| {
        if native.insert(id, value.clone()).is_none() {
            valued_assets.push(id);
        }
    };
    for row in valuations {
        if row.as_of_date < first_day {
            set_native(&mut native, row.asset_id, &row.value);
        }
    }
    let mut money = Vec::new();
    for day in days(first_day, today) {
        for (asset_id, _, currency) in &meta {
            if let Some(value) = values_on_day.get(&(day, *asset_id)) {
                set_native(&mut native, *asset_id, value);
            }
            if let Some(value) = native.get(asset_id) {
                money.push(MoneyAmount::new((day, *asset_id), value.clone(), *currency, day));
            }
        }
    }
    Some(AssetPlan {
        first_day,
        today,
        assets: meta.iter().map(|(id, class, _)| (*id, *class)).collect(),
        valued_assets,
        money,
    })
}

impl AssetPlan {
    /// Second half of `_asset_events_and_class_totals`.
    pub fn totals(&self, converted: &HashMap<(NaiveDate, i32), Decimal>) -> AssetTotals {
        // total_by_day[day] += value, iterating the converted dict, i.e. in
        // `money` order.
        let mut total_by_day: OrderedSums<NaiveDate> = OrderedSums::new();
        for item in &self.money {
            if let Some(value) = converted.get(&item.key) {
                total_by_day.add(item.key.0, value);
            }
        }
        let events = days(self.first_day, self.today).map(|day| (day, total_by_day.get(&day))).collect();

        let current_by_asset: Vec<(i32, Decimal)> = self
            .valued_assets
            .iter()
            .filter_map(|id| converted.get(&(self.today, *id)).map(|value| (*id, value.clone())))
            .collect();
        let class_of: HashMap<i32, AssetClass> = self.assets.iter().copied().collect();
        let mut class_totals: OrderedSums<AssetClass> = OrderedSums::new();
        for (asset_id, value) in &current_by_asset {
            if let Some(class) = class_of.get(asset_id) {
                class_totals.add(*class, value);
            }
        }
        let class_totals = class_totals.keys.iter().map(|class| (*class, class_totals.sums[class].clone())).collect();
        AssetTotals { events, class_totals, current_by_asset }
    }
}

// ---------------------------------------------------------------------------
// Cross-cuts
// ---------------------------------------------------------------------------

/// One `select(Asset.id, Asset.capital_role, Asset.monthly_cash_flow, Asset.currency)` row.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct CapitalRoleRow {
    pub id: i32,
    pub capital_role: CapitalRole,
    pub monthly_cash_flow: Option<Decimal>,
    pub currency: String,
}

/// The monthly cash flows to convert (keyed by row index):
/// `monthly_cash_flow or Decimal("0")` — a zero flow is falsy too.
pub fn capital_role_money(rows: &[CapitalRoleRow], today: NaiveDate) -> Vec<MoneyAmount<usize>> {
    rows.iter()
        .enumerate()
        .map(|(index, row)| {
            let flow = row.monthly_cash_flow.clone().filter(|f| !f.is_zero()).unwrap_or_else(Decimal::zero);
            MoneyAmount::new(index, flow, row.currency.clone(), today)
        })
        .collect()
}

/// `_capital_role_summary`: cross-cuts the same assets by how the user
/// tagged them (income / neutral / drain) instead of by asset class —
/// always all three roles, even at zero, so the block reads as a fixed
/// scale rather than a list that shuffles as assets are added.
pub fn capital_role_summary(
    rows: &[CapitalRoleRow],
    converted_flows: &HashMap<usize, Decimal>,
    assets: &AssetTotals,
) -> Vec<CapitalRoleSummary> {
    let mut totals_value: OrderedSums<CapitalRole> = OrderedSums::new();
    let mut totals_flow: OrderedSums<CapitalRole> = OrderedSums::new();
    let mut counts: HashMap<CapitalRole, i64> = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        totals_value.add(row.capital_role, &assets.current_value(row.id));
        totals_flow.add(row.capital_role, converted_flows.get(&index).unwrap_or(&Decimal::zero()));
        *counts.entry(row.capital_role).or_insert(0) += 1;
    }
    CapitalRole::ALL
        .iter()
        .map(|role| {
            let (label, color) = role_meta(*role);
            CapitalRoleSummary {
                role: role.as_str().to_string(),
                label: label.to_string(),
                color: color.to_string(),
                total_value: totals_value.get(role),
                monthly_cash_flow: totals_flow.get(role),
                count: counts.get(role).copied().unwrap_or(0),
            }
        })
        .collect()
}

/// One `select(Asset.id, Asset.name, Asset.risk_level)` row.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct RiskRow {
    pub id: i32,
    pub name: String,
    pub risk_level: RiskLevel,
}

/// `_risk_level_summary`: cross-cuts Cash + assets by user-tagged risk of
/// loss — unlike capital_roles, Cash participates here: it's the zero-risk
/// anchor an 80/20-style allocation rule ("80% of capital at zero risk, at
/// most 20% exposed") is measured against. Always all three tiers, even at
/// zero, same reasoning as capital_roles. Each tier's item list *is* its
/// diversification view — a tier that's one holding at 100% is
/// concentrated, several even-sized holdings aren't, no separate index.
pub fn risk_level_summary(rows: &[RiskRow], assets: &AssetTotals, cash_today: &Decimal) -> Vec<RiskLevelSummary> {
    let mut totals: OrderedSums<RiskLevel> = OrderedSums::new();
    let mut items_by_level: HashMap<RiskLevel, Vec<(String, String, Decimal)>> = HashMap::new();

    if !cash_today.is_zero() {
        totals.add(RiskLevel::Low, cash_today);
        items_by_level.entry(RiskLevel::Low).or_default().push((
            CASH_KEY.to_string(),
            class_meta(CASH_KEY).0.to_string(),
            cash_today.clone(),
        ));
    }
    for row in rows {
        let value = assets.current_value(row.id);
        if value.is_zero() {
            continue;
        }
        totals.add(row.risk_level, &value);
        items_by_level.entry(row.risk_level).or_default().push((format!("asset:{}", row.id), row.name.clone(), value));
    }
    let grand_total = totals.total();

    RiskLevel::ALL
        .iter()
        .map(|level| {
            let tier_total = totals.get(level);
            let mut items = items_by_level.remove(level).unwrap_or_default();
            // sorted(..., key=amount, reverse=True) — stable, amount desc.
            items.sort_by(|a, b| b.2.cmp(&a.2));
            let (label, color) = risk_meta(*level);
            RiskLevelSummary {
                risk_level: level.as_str().to_string(),
                label: label.to_string(),
                color: color.to_string(),
                percent: PyFloat(py_percent(&tier_total, &grand_total)),
                items: items
                    .into_iter()
                    .map(|(key, name, amount)| RiskLevelItem {
                        key,
                        name,
                        percent: PyFloat(py_percent(&amount, &tier_total)),
                        amount,
                    })
                    .collect(),
                total_value: tier_total,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Assembly
// ---------------------------------------------------------------------------

/// `_daily_series`: forward-fills a sparse, date-sorted list of cumulative
/// totals into one point per calendar day. `events` before `start` still
/// count — they just collapse into `start`'s opening value.
pub fn daily_series(events: &[(NaiveDate, Decimal)], start: NaiveDate, end: NaiveDate) -> Vec<NetWorthPoint> {
    let mut sorted: Vec<&(NaiveDate, Decimal)> = events.iter().collect();
    sorted.sort_by_key(|e| e.0);
    let mut points = Vec::new();
    let mut idx = 0;
    let mut current = Decimal::zero();
    for day in days(start, end) {
        while idx < sorted.len() && sorted[idx].0 <= day {
            current = sorted[idx].1.clone();
            idx += 1;
        }
        points.push(NetWorthPoint { date: day, value: current.clone() });
    }
    points
}

/// `_resolve_start_date`: a fixed range starts `days - 1` days ago, `"all"`
/// at the earliest event (or today when there is none).
pub fn resolve_start_date(
    range_key: &str,
    cash_events: &[(NaiveDate, Decimal)],
    asset_events: &[(NaiveDate, Decimal)],
    today: NaiveDate,
) -> NaiveDate {
    if let Some(start) = requested_start(range_key, today) {
        return start;
    }
    cash_events.iter().chain(asset_events).map(|e| e.0).min().unwrap_or(today)
}

/// `cash_events[-1][1] if cash_events else Decimal("0")` — the events are
/// cumulative, so the last one *is* today's total.
pub fn cash_today(cash_events: &[(NaiveDate, Decimal)]) -> Decimal {
    cash_events.last().map_or_else(Decimal::zero, |e| e.1.clone())
}

/// The tail of `get_net_worth_summary`: series, change and the breakdown.
pub fn assemble_summary(
    range_key: &str,
    today: NaiveDate,
    cash_events: &[(NaiveDate, Decimal)],
    assets: &AssetTotals,
    capital_roles: Vec<CapitalRoleSummary>,
    risk_levels: Vec<RiskLevelSummary>,
) -> NetWorthSummary {
    let start = resolve_start_date(range_key, cash_events, &assets.events, today);
    let cash_series = daily_series(cash_events, start, today);
    let asset_series = daily_series(&assets.events, start, today);
    let series: Vec<NetWorthPoint> = cash_series
        .into_iter()
        .zip(asset_series)
        .map(|(c, a)| NetWorthPoint { date: c.date, value: c.value.add(&a.value) })
        .collect();

    let current = series.last().map_or_else(Decimal::zero, |p| p.value.clone());
    let start_value = series.first().map_or_else(Decimal::zero, |p| p.value.clone());
    let change_amount = current.sub(&start_value);
    let change_percent =
        if start_value.is_zero() { None } else { Some(PyFloat(py_percent(&change_amount, &start_value))) };

    let cash = cash_today(cash_events);
    let total = cash.add(&assets.classes_sum());
    let mut breakdown = Vec::with_capacity(AssetClass::ALL.len() + 1);
    let item = |key: &str, amount: Decimal| {
        let (name, color, icon) = class_meta(key);
        NetWorthBreakdownItem {
            key: key.to_string(),
            name: name.to_string(),
            color: color.to_string(),
            icon: icon.to_string(),
            percent: PyFloat(py_percent(&amount, &total)),
            amount,
        }
    };
    breakdown.push(item(CASH_KEY, cash));
    for asset_class in AssetClass::ALL {
        breakdown.push(item(asset_class.as_str(), assets.class_total(*asset_class)));
    }

    NetWorthSummary {
        range: range_key.to_string(),
        current,
        change_amount,
        change_percent,
        series,
        breakdown,
        capital_roles,
        risk_levels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn dec(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn identity(money: &[MoneyAmount<(NaiveDate, i32)>]) -> HashMap<(NaiveDate, i32), Decimal> {
        money.iter().map(|m| (m.key, m.amount.clone())).collect()
    }

    #[test]
    fn percent_keeps_python_sign_of_zero() {
        assert_eq!(py_percent(&dec("25"), &dec("100")), 25.0);
        assert!(py_percent(&Decimal::zero(), &dec("-50")).is_sign_negative());
        assert!(!py_percent(&Decimal::zero(), &dec("50")).is_sign_negative());
        assert_eq!(py_percent(&dec("1"), &Decimal::zero()), 0.0);
    }

    #[test]
    fn cash_series_forward_fills_balances() {
        let accounts = vec![
            CashAccountRow { id: 1, type_: AccountType::Checking, currency: "USD".into() },
            CashAccountRow { id: 2, type_: AccountType::CreditCard, currency: "USD".into() },
        ];
        let tx = |d: NaiveDate, type_, amount: &str, account_id, to: Option<i32>| CashTransactionRow {
            date: d,
            type_,
            amount: dec(amount),
            destination_amount: None,
            account_id,
            transfer_account_id: to,
        };
        let transactions = vec![
            tx(day(2026, 1, 1), TransactionType::Income, "100.00", 1, None),
            tx(day(2026, 1, 3), TransactionType::Expense, "30.00", 1, None),
            tx(day(2026, 1, 3), TransactionType::Expense, "500.00", 2, None),
            tx(day(2026, 1, 3), TransactionType::Transfer, "10.00", 2, Some(1)),
        ];
        let plan = cash_plan(&accounts, &transactions, Some(day(2026, 1, 2)), day(2026, 1, 4)).unwrap();
        assert_eq!(plan.first_day, day(2026, 1, 2));
        let events = plan.events(&identity(&plan.money));
        let values: Vec<String> = events.iter().map(|e| e.1.to_string()).collect();
        assert_eq!(values, vec!["100.00", "80.00", "80.00"]);
        assert!(cash_plan(&accounts, &[], None, day(2026, 1, 4)).is_none());
    }

    #[test]
    fn assets_revalue_latest_known_value() {
        let assets = vec![
            AssetMetaRow { id: 7, asset_class: AssetClass::RealEstate, currency: "USD".into() },
            AssetMetaRow { id: 8, asset_class: AssetClass::Crypto, currency: "USD".into() },
        ];
        let valuations = vec![
            ValuationRow { asset_id: 7, as_of_date: day(2026, 1, 1), value: dec("1000.00") },
            ValuationRow { asset_id: 8, as_of_date: day(2026, 1, 3), value: dec("50.00") },
            ValuationRow { asset_id: 7, as_of_date: day(2026, 1, 3), value: dec("1100.00") },
        ];
        let plan = asset_plan(&assets, &valuations, Some(day(2026, 1, 2)), day(2026, 1, 3)).unwrap();
        assert_eq!(plan.valued_assets, vec![7, 8]);
        let totals = plan.totals(&identity(&plan.money));
        let values: Vec<String> = totals.events.iter().map(|e| e.1.to_string()).collect();
        assert_eq!(values, vec!["1000.00", "1150.00"]);
        assert_eq!(totals.class_total(AssetClass::Crypto).to_string(), "50.00");
        assert_eq!(totals.class_total(AssetClass::Vehicles).to_string(), "0");
    }

    #[test]
    fn summary_breakdown_and_tiers() {
        let today = day(2026, 1, 3);
        let cash_events = vec![(day(2026, 1, 2), dec("100.00")), (today, dec("50.00"))];
        let assets = AssetTotals {
            events: vec![(today, dec("150.00"))],
            class_totals: vec![(AssetClass::Investments, dec("150.00"))],
            current_by_asset: vec![(1, dec("150.00"))],
        };
        let rows = vec![RiskRow { id: 1, name: "ETF".into(), risk_level: RiskLevel::High }];
        let tiers = risk_level_summary(&rows, &assets, &dec("50.00"));
        assert_eq!(tiers[0].percent, PyFloat(25.0));
        assert_eq!(tiers[1].total_value.to_string(), "0");
        assert_eq!(tiers[2].items[0].key, "asset:1");
        let summary = assemble_summary("all", today, &cash_events, &assets, Vec::new(), tiers);
        assert_eq!(summary.series.len(), 2);
        assert_eq!(summary.current.to_string(), "200.00");
        assert_eq!(summary.change_amount.to_string(), "100.00");
        assert_eq!(summary.change_percent, Some(PyFloat(100.0)));
        assert_eq!(summary.breakdown[1].percent, PyFloat(75.0));
        assert_eq!(summary.breakdown.len(), 7);
    }
}
