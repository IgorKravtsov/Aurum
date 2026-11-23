//! Tags: CRUD, case-insensitive dedup on create, and their use on
//! transactions (create/update with tag_ids, filtering by tag_id).

use serde_json::{Value, json};

use crate::common::{TestApp, txn_payload as txn};

fn tag_ids(transaction: &Value) -> Vec<Value> {
    transaction["tags"].as_array().unwrap().iter().map(|t| t["id"].clone()).collect()
}

#[tokio::test]
async fn test_create_tag() {
    let client = TestApp::start().await;
    let resp = client.post("/tags", json!({"name": "Georgia Trip"})).await;
    assert_eq!(resp.status, 201);
    assert_eq!(resp.json()["name"], "Georgia Trip");
}

#[tokio::test]
async fn test_create_tag_dedups_case_insensitively() {
    let client = TestApp::start().await;
    let first = client.post("/tags", json!({"name": "Tax Deductible"})).await;
    let second = client.post("/tags", json!({"name": "tax deductible"})).await;

    assert_eq!(first.json()["id"], second.json()["id"]);
    assert_eq!(client.get("/tags").await.json().as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_delete_tag() {
    let client = TestApp::start().await;
    let created = client.post("/tags", json!({"name": "Temp"})).await;
    let resp = client.delete(&format!("/tags/{}", created.json()["id"])).await;
    assert_eq!(resp.status, 204);
    assert_eq!(client.get("/tags").await.json(), json!([]));
}

#[tokio::test]
async fn test_create_transaction_with_tags() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let tag = client.post("/tags", json!({"name": "Georgia"})).await;
    let tag_id = tag.json()["id"].clone();

    let resp = client
        .post("/transactions", txn(account_id, json!({"category_id": categories.id("Groceries"), "tag_ids": [tag_id]})))
        .await;
    assert_eq!(resp.status, 201);
    assert_eq!(tag_ids(&resp.json()), vec![tag_id]);
}

#[tokio::test]
async fn test_update_transaction_replaces_tags() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let tag_a = client.post("/tags", json!({"name": "A"})).await.json()["id"].clone();
    let tag_b = client.post("/tags", json!({"name": "B"})).await.json()["id"].clone();
    let created = client
        .post("/transactions", txn(account_id, json!({"category_id": categories.id("Groceries"), "tag_ids": [tag_a]})))
        .await;
    let txn_id = created.json()["id"].clone();

    let resp = client.patch(&format!("/transactions/{txn_id}"), json!({"tag_ids": [tag_b]})).await;
    assert_eq!(tag_ids(&resp.json()), vec![tag_b.clone()]);

    // Omitting tag_ids entirely leaves the existing tags untouched.
    let resp = client.patch(&format!("/transactions/{txn_id}"), json!({"description": "renamed"})).await;
    assert_eq!(tag_ids(&resp.json()), vec![tag_b]);

    // Sending an empty list clears them.
    let resp = client.patch(&format!("/transactions/{txn_id}"), json!({"tag_ids": []})).await;
    assert_eq!(resp.json()["tags"], json!([]));
}

#[tokio::test]
async fn test_create_transaction_rejects_unknown_tag_id() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let resp = client
        .post("/transactions", txn(account_id, json!({"category_id": categories.id("Groceries"), "tag_ids": [999999]})))
        .await;
    assert_eq!(resp.status, 400);
}

#[tokio::test]
async fn test_filter_transactions_by_tag() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let tag = client.post("/tags", json!({"name": "Filtered"})).await.json()["id"].clone();
    client
        .post("/transactions", txn(account_id, json!({"category_id": categories.id("Groceries"), "tag_ids": [tag]})))
        .await;
    client.post("/transactions", txn(account_id, json!({"category_id": categories.id("Groceries")}))).await;

    let resp = client.get_with("/transactions", json!({"tag_id": tag})).await;
    let body = resp.json();
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["tags"][0]["id"], tag);
}
