//! Category subcategories (parent_id): creation rules and the one-level-deep
//! invariant enforced in routes/categories.rs. Also default-category
//! deletion: a seeded category can be removed once nothing references it, but
//! never while it still has transactions.

use serde_json::{Value, json};

use crate::common::{TestApp, txn_payload as txn};

fn ids(listing: &Value) -> Vec<Value> {
    listing.as_array().unwrap().iter().map(|c| c["id"].clone()).collect()
}

#[tokio::test]
async fn test_create_subcategory_under_top_level_category() {
    let client = TestApp::start().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let resp = client
        .post("/categories", json!({"name": "Alcohol", "kind": "expense", "color": "#e34948", "parent_id": groceries}))
        .await;
    assert_eq!(resp.status, 201);
    assert_eq!(resp.json()["parent_id"], groceries);
}

#[tokio::test]
async fn test_subcategory_cannot_have_a_different_kind_than_its_parent() {
    let client = TestApp::start().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let resp = client
        .post("/categories", json!({"name": "Bad", "kind": "income", "color": "#e34948", "parent_id": groceries}))
        .await;
    assert_eq!(resp.status, 400);
}

#[tokio::test]
async fn test_subcategories_are_only_one_level_deep() {
    let client = TestApp::start().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let child = client
        .post("/categories", json!({"name": "Alcohol", "kind": "expense", "color": "#e34948", "parent_id": groceries}))
        .await;
    let child_id = child.json()["id"].clone();

    let resp = client
        .post("/categories", json!({"name": "Wine", "kind": "expense", "color": "#e34948", "parent_id": child_id}))
        .await;
    assert_eq!(resp.status, 400);
}

#[tokio::test]
async fn test_category_cannot_be_its_own_parent() {
    let client = TestApp::start().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let resp = client.patch(&format!("/categories/{groceries}"), json!({"parent_id": groceries})).await;
    assert_eq!(resp.status, 400);
}

#[tokio::test]
async fn test_a_category_with_subcategories_cannot_itself_become_a_subcategory() {
    let client = TestApp::start().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let dining = categories.id("Dining Out");
    client
        .post("/categories", json!({"name": "Alcohol", "kind": "expense", "color": "#e34948", "parent_id": groceries}))
        .await;

    let resp = client.patch(&format!("/categories/{groceries}"), json!({"parent_id": dining})).await;
    assert_eq!(resp.status, 400);
}

#[tokio::test]
async fn test_deleting_a_parent_leaves_children_as_top_level() {
    let client = TestApp::start().await;
    // A default category with transactions can't be deleted (separate rule,
    // tested below) — use a fresh, non-default parent so this test isolates
    // the parent/child rule.
    let parent =
        client.post("/categories", json!({"name": "Custom Parent", "kind": "expense", "color": "#e34948"})).await;
    let parent_id = parent.json()["id"].clone();
    let created = client
        .post("/categories", json!({"name": "Alcohol", "kind": "expense", "color": "#e34948", "parent_id": parent_id}))
        .await;
    let child_id = created.json()["id"].clone();

    let resp = client.delete(&format!("/categories/{parent_id}")).await;
    assert_eq!(resp.status, 204);

    let refetched = client.get("/categories").await.json();
    let child = refetched.as_array().unwrap().iter().find(|c| c["id"] == child_id).unwrap();
    assert!(child["parent_id"].is_null());
}

#[tokio::test]
async fn test_default_category_with_no_transactions_can_be_deleted() {
    let client = TestApp::start().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    assert_eq!(categories.get("Groceries")["is_default"], json!(true));

    let resp = client.delete(&format!("/categories/{groceries}")).await;
    assert_eq!(resp.status, 204);

    let refetched = client.get("/categories").await.json();
    assert!(!ids(&refetched).contains(&json!(groceries)));
}

#[tokio::test]
async fn test_default_category_with_a_transaction_cannot_be_deleted() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    client.post("/transactions", txn(account_id, json!({"category_id": groceries}))).await;

    let resp = client.delete(&format!("/categories/{groceries}")).await;
    assert_eq!(resp.status, 400);

    let refetched = client.get("/categories").await.json();
    assert!(ids(&refetched).contains(&json!(groceries)));
}

#[tokio::test]
async fn test_default_category_used_only_by_a_split_line_cannot_be_deleted() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
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
                    "splits": [
                        {"category_id": groceries, "amount": "70.00"},
                        {"category_id": sweets, "amount": "30.00"},
                    ],
                }),
            ),
        )
        .await;

    let resp = client.delete(&format!("/categories/{groceries}")).await;
    assert_eq!(resp.status, 400);
}

#[tokio::test]
async fn test_default_category_becomes_deletable_once_its_transaction_is_removed() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let created = client.post("/transactions", txn(account_id, json!({"category_id": groceries}))).await;
    let txn_id = created.json()["id"].clone();

    let blocked = client.delete(&format!("/categories/{groceries}")).await;
    assert_eq!(blocked.status, 400);

    assert_eq!(client.delete(&format!("/transactions/{txn_id}")).await.status, 204);

    let resp = client.delete(&format!("/categories/{groceries}")).await;
    assert_eq!(resp.status, 204);
}
