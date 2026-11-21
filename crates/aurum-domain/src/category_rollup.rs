//! Spend/income per top-level category, split-aware — the pure half of
//! `backend/app/services/category_rollup.py`, shared by the dashboard's
//! breakdown, the reports' category ranking, budgets and insights: every
//! place that rolls a subcategory's amount up into its parent
//! (`coalesce(parent_id, id)`) and needs a category's share of a split
//! transaction counted the same way a plain transaction's is.
//!
//! A split transaction has `category_id = NULL` (the categories live on its
//! split lines) — a query that only joins on that column would silently
//! drop its spending from every category it actually touches. The server's
//! `services::category_rollup` walks both sources (a transaction's own
//! category_id and its splits' category_ids), converts them into the display
//! currency and hands the [`CategoryContribution`]s to [`rollup`].

use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;

use crate::decimal::Decimal;

/// Fallback color of a category that no longer exists.
pub const UNKNOWN_CATEGORY_COLOR: &str = "#898781";
/// Fallback name of a category that no longer exists.
pub const UNKNOWN_CATEGORY_NAME: &str = "?";

/// One converted contribution: a plain transaction contributes one, a split
/// one per split line. `amount` is already in `currency`, the app's
/// display currency.
#[derive(Debug, Clone, PartialEq)]
pub struct CategoryContribution {
    pub transaction_id: i32,
    pub category_id: i32,
    pub amount: Decimal,
    pub transaction_date: NaiveDate,
    pub currency: String,
}

/// The category columns the rollup needs (`select(Category)`).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct RollupCategory {
    pub id: i32,
    pub name: String,
    pub color: String,
    pub icon: Option<String>,
    pub sort_order: i32,
    pub parent_id: Option<i32>,
}

/// One leaf category's share of its parent's total — a real subcategory, or
/// the parent category itself when some of the spend was filed directly
/// under it with no more specific subcategory (`category_id` equals the
/// enclosing [`CategoryRollupItem::category_id`] in that case).
#[derive(Debug, Clone, PartialEq)]
pub struct CategoryRollupChildItem {
    pub category_id: i32,
    pub name: String,
    pub color: String,
    pub icon: Option<String>,
    pub amount: Decimal,
}

/// One top-level category's total.
#[derive(Debug, Clone, PartialEq)]
pub struct CategoryRollupItem {
    pub category_id: i32,
    pub name: String,
    pub color: String,
    pub icon: Option<String>,
    pub sort_order: i32,
    pub amount: Decimal,
    /// Distinct transactions contributing to this category — a split
    /// transaction counts once per top-level category it touches, same as a
    /// plain one counts once for its own category.
    pub transaction_count: usize,
    /// Populated only when more than one distinct category actually fed this
    /// total (a subcategory, or a direct-to-parent line, or several of
    /// either) — e.g. a receipt split between "Sweets" and "Alcohol" under
    /// "Groceries". A single-source category (the common case) leaves this
    /// empty rather than showing a redundant one-item breakdown of itself.
    pub children: Vec<CategoryRollupChildItem>,
}

/// A Python `dict` with `defaultdict(Decimal)` semantics: insertion-ordered,
/// every value starting at `Decimal("0")`.
#[derive(Debug, Default)]
struct OrderedSums {
    keys: Vec<i32>,
    sums: HashMap<i32, Decimal>,
}

impl OrderedSums {
    fn add(&mut self, key: i32, amount: &Decimal) {
        let sum = self.sums.entry(key).or_insert_with(|| {
            self.keys.push(key);
            Decimal::zero()
        });
        *sum = sum.add(amount);
    }

    fn iter(&self) -> impl Iterator<Item = (i32, &Decimal)> {
        self.keys.iter().map(|k| (*k, &self.sums[k]))
    }

    fn len(&self) -> usize {
        self.keys.len()
    }
}

/// Every top-level category's total, sorted by amount desc (category
/// `sort_order` as tiebreak; ties beyond that keep first-contribution
/// order, as Python's stable sort over dict order does). Port of the
/// in-memory half of `rollup_spending_by_top_level_category`.
pub fn rollup(contributions: &[CategoryContribution], categories: &[RollupCategory]) -> Vec<CategoryRollupItem> {
    let by_id: HashMap<i32, &RollupCategory> = categories.iter().map(|c| (c.id, c)).collect();

    let mut amount_by_effective = OrderedSums::default();
    let mut txn_ids_by_effective: HashMap<i32, HashSet<i32>> = HashMap::new();
    // effective (top-level) category id -> {actual leaf category_id: amount}
    // — the leaf is the same as the effective id when a contribution was
    // filed directly on the parent, and a genuine child id otherwise.
    let mut amount_by_leaf: HashMap<i32, OrderedSums> = HashMap::new();
    for contribution in contributions {
        let effective_id =
            by_id.get(&contribution.category_id).and_then(|c| c.parent_id).unwrap_or(contribution.category_id);
        amount_by_effective.add(effective_id, &contribution.amount);
        amount_by_leaf.entry(effective_id).or_default().add(contribution.category_id, &contribution.amount);
        txn_ids_by_effective.entry(effective_id).or_default().insert(contribution.transaction_id);
    }

    let mut items: Vec<CategoryRollupItem> = Vec::with_capacity(amount_by_effective.len());
    for (effective_id, amount) in amount_by_effective.iter() {
        let category = by_id.get(&effective_id);
        let leaves = &amount_by_leaf[&effective_id];
        let mut children = Vec::new();
        if leaves.len() > 1 {
            let mut leaf_amounts: Vec<(i32, &Decimal)> = leaves.iter().collect();
            // sorted(..., key=lambda pair: -pair[1]) — stable, amount desc.
            leaf_amounts.sort_by(|a, b| b.1.cmp(a.1));
            for (leaf_id, leaf_amount) in leaf_amounts {
                let leaf = by_id.get(&leaf_id);
                children.push(CategoryRollupChildItem {
                    category_id: leaf_id,
                    name: leaf.map_or(UNKNOWN_CATEGORY_NAME, |c| c.name.as_str()).to_string(),
                    color: leaf.map_or(UNKNOWN_CATEGORY_COLOR, |c| c.color.as_str()).to_string(),
                    icon: leaf.and_then(|c| c.icon.clone()),
                    amount: leaf_amount.clone(),
                });
            }
        }
        items.push(CategoryRollupItem {
            category_id: effective_id,
            name: category.map_or(UNKNOWN_CATEGORY_NAME, |c| c.name.as_str()).to_string(),
            color: category.map_or(UNKNOWN_CATEGORY_COLOR, |c| c.color.as_str()).to_string(),
            icon: category.and_then(|c| c.icon.clone()),
            sort_order: category.map_or(0, |c| c.sort_order),
            amount: amount.clone(),
            transaction_count: txn_ids_by_effective[&effective_id].len(),
            children,
        });
    }
    items.sort_by(|a, b| b.amount.cmp(&a.amount).then(a.sort_order.cmp(&b.sort_order)));
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn category(id: i32, parent_id: Option<i32>, sort_order: i32) -> RollupCategory {
        RollupCategory { id, name: format!("c{id}"), color: "#000000".into(), icon: None, sort_order, parent_id }
    }

    fn contribution(transaction_id: i32, category_id: i32, amount: &str) -> CategoryContribution {
        CategoryContribution {
            transaction_id,
            category_id,
            amount: amount.parse().unwrap(),
            transaction_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            currency: "USD".into(),
        }
    }

    #[test]
    fn rolls_children_into_parents() {
        let categories = vec![category(1, None, 1), category(2, Some(1), 0), category(3, None, 0)];
        let contributions = vec![
            contribution(10, 2, "3.50"),
            contribution(10, 1, "6.50"),
            contribution(11, 3, "10.00"),
            contribution(12, 9, "1.00"),
        ];
        let items = rollup(&contributions, &categories);
        // 10.00 vs 10.00: sort_order 0 (category 3) wins the tie.
        assert_eq!(items.iter().map(|i| i.category_id).collect::<Vec<_>>(), vec![3, 1, 9]);
        let groceries = &items[1];
        assert_eq!(groceries.amount.to_string(), "10.00");
        assert_eq!(groceries.transaction_count, 1);
        assert_eq!(groceries.children.iter().map(|c| c.category_id).collect::<Vec<_>>(), vec![1, 2]);
        assert!(items[0].children.is_empty());
        assert_eq!(items[2].name, "?");
        assert_eq!(items[2].color, UNKNOWN_CATEGORY_COLOR);
    }
}
