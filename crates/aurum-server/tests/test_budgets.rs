//! Budgets: category-kind restriction, one-budget-per-category, and the
//! spent/remaining/over-budget math behind the Budget page's progress bars.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::common::{Api, TestApp, dec, money, txn_payload};

#[tokio::test]
async fn test_budget_rejects_income_category() {
    let client = TestApp::start().await;
    let categories = client.categories().await;
    let resp =
        client.post("/budgets", json!({"category_id": categories.id("Salary"), "monthly_limit": "500.00"})).await;
    assert_eq!(resp.status, 400);
}

#[tokio::test]
async fn test_budget_rejects_duplicate_category() {
    let client = TestApp::start().await;
    let categories = client.categories().await;
    let category_id = categories.id("Groceries");
    let first = client.post("/budgets", json!({"category_id": category_id, "monthly_limit": "300.00"})).await;
    assert_eq!(first.status, 201);

    let second = client.post("/budgets", json!({"category_id": category_id, "monthly_limit": "400.00"})).await;
    assert_eq!(second.status, 400);
}

#[tokio::test]
async fn test_status_reports_spent_remaining_and_over_budget() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let category_id = categories.id("Groceries");
    client.post("/budgets", json!({"category_id": category_id, "monthly_limit": "100.00"})).await;

    for amount in ["60.00", "60.00"] {
        // 120.00 total, over a 100.00 limit
        client
            .post(
                "/transactions",
                txn_payload(account_id, json!({"amount": amount, "category_id": category_id, "date": "2026-08-10"})),
            )
            .await;
    }

    let resp = client.get_with("/budgets/status", json!({"year": 2026, "month": 8})).await;
    let item = resp.json()["items"][0].clone();
    assert_eq!(money(&item["spent"]), dec("120.00"));
    assert_eq!(money(&item["remaining"]), dec("-20.00"));
    assert_eq!(item["percent"].as_f64(), Some(120.0));
    assert_eq!(item["is_over_budget"], json!(true));
}

#[tokio::test]
async fn test_status_only_counts_the_selected_month() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let category_id = categories.id("Groceries");
    client.post("/budgets", json!({"category_id": category_id, "monthly_limit": "100.00"})).await;
    client
        .post(
            "/transactions",
            txn_payload(account_id, json!({"amount": "90.00", "category_id": category_id, "date": "2026-07-15"})),
        )
        .await;

    let resp = client.get_with("/budgets/status", json!({"year": 2026, "month": 8})).await;
    let item = resp.json()["items"][0].clone();
    assert_eq!(money(&item["spent"]), dec("0"));
    assert_eq!(item["is_over_budget"], json!(false));
}

#[tokio::test]
async fn test_status_is_empty_list_when_no_budgets_exist() {
    let client = TestApp::start().await;
    let resp = client.get_with("/budgets/status", json!({"year": 2026, "month": 8})).await;
    assert_eq!(resp.json()["items"], json!([]));
}

#[tokio::test]
async fn test_update_budget_changes_the_limit() {
    let client = TestApp::start().await;
    let categories = client.categories().await;
    let category_id = categories.id("Groceries");
    let created = client.post("/budgets", json!({"category_id": category_id, "monthly_limit": "100.00"})).await;
    let budget_id = created.json()["id"].clone();

    let resp = client.patch(&format!("/budgets/{budget_id}"), json!({"monthly_limit": "250.00"})).await;
    assert_eq!(resp.status, 200);
    assert_eq!(money(&resp.json()["monthly_limit"]), dec("250.00"));
}

#[tokio::test]
async fn test_delete_budget_removes_it_from_status() {
    let client = TestApp::start().await;
    let categories = client.categories().await;
    let category_id = categories.id("Groceries");
    let created = client.post("/budgets", json!({"category_id": category_id, "monthly_limit": "100.00"})).await;
    let budget_id = created.json()["id"].clone();

    let delete_resp = client.delete(&format!("/budgets/{budget_id}")).await;
    assert_eq!(delete_resp.status, 204);

    let status = client.get_with("/budgets/status", json!({"year": 2026, "month": 8})).await;
    assert_eq!(status.json()["items"], json!([]));
}

async fn subcategory(client: &Api, parent_id: i64, name: &str) -> i64 {
    let resp = client
        .post("/categories", json!({"name": name, "kind": "expense", "color": "#7a869a", "parent_id": parent_id}))
        .await;
    resp.json()["id"].as_i64().unwrap()
}

fn items_by_category(status: &Value) -> HashMap<i64, Value> {
    status["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| (item["category_id"].as_i64().unwrap(), item.clone()))
        .collect()
}

/// The Dashboard breakdown and the category report both roll a
/// subcategory's spending up into its parent (coalesce(parent_id, id)). A
/// budget set on the parent has to agree with them, or the same month reads
/// as 900 spent on one screen and 0 on the other.
#[tokio::test]
async fn test_budget_on_a_parent_category_counts_spending_in_its_subcategories() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let parent_id = categories.id("Housing & Utilities");
    let child_id = subcategory(&client, parent_id, "Rent").await;
    client.post("/budgets", json!({"category_id": parent_id, "monthly_limit": "1000.00"})).await;
    client
        .post(
            "/transactions",
            txn_payload(account_id, json!({"category_id": child_id, "amount": "900.00", "date": "2026-03-10"})),
        )
        .await;

    let status = client.get("/budgets/status?year=2026&month=3").await.json()["items"][0].clone();
    let dashboard = client.get("/dashboard/summary?year=2026&month=3").await.json();

    assert_eq!(money(&status["spent"]), dec("900.00"));
    assert_eq!(money(&status["remaining"]), dec("100.00"));
    // The two screens must show the same number for the same category.
    let parent_slice =
        dashboard["spending_by_category"].as_array().unwrap().iter().find(|s| s["category_id"] == parent_id).unwrap();
    assert_eq!(money(&parent_slice["amount"]), money(&status["spent"]));
}

/// Rolling children into a parent's budget must not swallow a budget set
/// on the child itself — both bars track the same spending, each against its
/// own limit.
#[tokio::test]
async fn test_a_subcategory_keeps_its_own_budget_separate() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let parent_id = categories.id("Housing & Utilities");
    let child_id = subcategory(&client, parent_id, "Rent").await;
    client.post("/budgets", json!({"category_id": parent_id, "monthly_limit": "1000.00"})).await;
    client.post("/budgets", json!({"category_id": child_id, "monthly_limit": "800.00"})).await;
    client
        .post(
            "/transactions",
            txn_payload(account_id, json!({"category_id": child_id, "amount": "900.00", "date": "2026-03-10"})),
        )
        .await;

    let items = items_by_category(&client.get("/budgets/status?year=2026&month=3").await.json());

    assert_eq!(money(&items[&child_id]["spent"]), dec("900.00"));
    assert_eq!(items[&child_id]["is_over_budget"], json!(true));
    assert_eq!(money(&items[&parent_id]["spent"]), dec("900.00"));
    assert_eq!(items[&parent_id]["is_over_budget"], json!(false));
}

/// A receipt split between Groceries and one of its own subcategories
/// (Sweets) must count each line toward the right budget: the full 100.00
/// rolls up into the Groceries budget, but only the 30.00 Sweets line
/// counts toward a budget set on Sweets itself.
#[tokio::test]
async fn test_budget_status_counts_a_split_transactions_share() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let sweets_resp = client
        .post("/categories", json!({"name": "Sweets", "kind": "expense", "color": "#7a869a", "parent_id": groceries}))
        .await;
    let sweets = sweets_resp.json()["id"].as_i64().unwrap();
    client.post("/budgets", json!({"category_id": groceries, "monthly_limit": "100.00"})).await;
    client.post("/budgets", json!({"category_id": sweets, "monthly_limit": "50.00"})).await;
    client
        .post(
            "/transactions",
            txn_payload(
                account_id,
                json!({
                    "amount": "100.00",
                    "category_id": null,
                    "date": "2026-08-10",
                    "splits": [
                        {"category_id": groceries, "amount": "70.00"},
                        {"category_id": sweets, "amount": "30.00"},
                    ],
                }),
            ),
        )
        .await;

    let resp = client.get_with("/budgets/status", json!({"year": 2026, "month": 8})).await;
    let items = items_by_category(&resp.json());
    assert_eq!(money(&items[&groceries]["spent"]), dec("100.00"));
    assert_eq!(money(&items[&sweets]["spent"]), dec("30.00"));
}
