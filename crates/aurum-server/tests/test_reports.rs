//! Long-range reports: the category ranking (which categories cost the
//! most over an arbitrary period) and the single-category spending detail —
//! both split-aware via services/category_rollup.rs, same as the Dashboard
//! breakdown (see test_dashboard.rs).

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::common::{Api, TestApp, dec, money, txn_payload as txn};

/// `{item["name"]: item for item in items}`.
fn by_name(rows: &Value) -> HashMap<String, Value> {
    rows.as_array().unwrap().iter().map(|row| (row["name"].as_str().unwrap().to_string(), row.clone())).collect()
}

/// Sweets under Groceries plus a 70/30 split receipt across the two.
async fn split_receipt(client: &Api, account_id: i64, groceries: i64) -> Value {
    let sweets = client
        .post("/categories", json!({"name": "Sweets", "kind": "expense", "color": "#7a869a", "parent_id": groceries}))
        .await
        .json()["id"]
        .clone();
    client
        .post(
            "/transactions",
            txn(
                account_id,
                json!({
                    "amount": "100.00",
                    "category_id": null,
                    "date": "2026-08-01",
                    "splits": [
                        {"category_id": groceries, "amount": "70.00"},
                        {"category_id": sweets, "amount": "30.00"},
                    ],
                }),
            ),
        )
        .await;
    sweets
}

#[tokio::test]
async fn test_ranking_rolls_up_a_split_into_one_item_with_children() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    split_receipt(&client, account_id, groceries).await;

    let resp = client.get_with("/reports/category-ranking", json!({"kind": "expense"})).await;
    let items = by_name(&resp.json()["items"]);

    assert!(!items.contains_key("Sweets"));
    assert_eq!(money(&items["Groceries"]["amount"]), dec("100.00"));
    let children = by_name(&items["Groceries"]["children"]);
    assert_eq!(money(&children["Groceries"]["amount"]), dec("70.00"));
    assert_eq!(money(&children["Sweets"]["amount"]), dec("30.00"));
}

#[tokio::test]
async fn test_ranking_item_with_a_single_source_has_no_children() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    client
        .post(
            "/transactions",
            txn(account_id, json!({"amount": "50.00", "category_id": groceries, "date": "2026-08-01"})),
        )
        .await;

    let resp = client.get_with("/reports/category-ranking", json!({"kind": "expense"})).await;
    let items = by_name(&resp.json()["items"]);

    assert_eq!(items["Groceries"]["children"], json!([]));
}

/// A top-level category's own report folds in both its plain
/// transactions and any split lines landing on it or its subcategories.
#[tokio::test]
async fn test_category_spending_report_counts_a_subcategorys_split_share() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let sweets = split_receipt(&client, account_id, groceries).await;

    let parent_report = client.get_with("/reports/category-spending", json!({"category_id": groceries})).await.json();
    assert_eq!(money(&parent_report["total_amount"]), dec("100.00"));
    assert_eq!(parent_report["transaction_count"], 1);

    let child_report = client.get_with("/reports/category-spending", json!({"category_id": sweets})).await.json();
    assert_eq!(money(&child_report["total_amount"]), dec("30.00"));
}
