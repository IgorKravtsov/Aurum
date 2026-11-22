//! Full-database backup export/import (services/backup.rs) — only
//! the parts this change touched: subcategories (self-referential parent_id)
//! and tags (many-to-many) surviving a round trip.

use serde_json::{Value, json};

use crate::common::{TestApp, txn_payload as txn};

#[tokio::test]
async fn test_backup_roundtrip_preserves_subcategories_and_tags() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let parent =
        client.post("/categories", json!({"name": "Custom Parent", "kind": "expense", "color": "#e34948"})).await;
    let parent_id = parent.json()["id"].clone();
    let child = client
        .post(
            "/categories",
            json!({"name": "Custom Child", "kind": "expense", "color": "#e34948", "parent_id": parent_id}),
        )
        .await;
    let child_id = child.json()["id"].clone();

    let tag = client.post("/tags", json!({"name": "Roundtrip"})).await.json()["id"].clone();
    let created =
        client.post("/transactions", txn(account_id, json!({"category_id": child_id, "tag_ids": [tag]}))).await;
    let txn_id = created.json()["id"].clone();

    let export_resp = client.get("/backup/export").await;
    assert_eq!(export_resp.status, 200);
    let payload = export_resp.json();

    let import_resp = client.post("/backup/import", payload).await;
    assert_eq!(import_resp.status, 200, "{}", import_resp.text);

    let refetched_categories = client.get("/categories").await.json();
    let child = refetched_categories.as_array().unwrap().iter().find(|c| c["id"] == child_id).unwrap();
    assert_eq!(child["parent_id"], parent_id);

    let listing = client.get("/transactions").await.json();
    let refetched_txn = listing["items"].as_array().unwrap().iter().find(|t| t["id"] == txn_id).unwrap();
    let tag_ids: Vec<&Value> = refetched_txn["tags"].as_array().unwrap().iter().map(|t| &t["id"]).collect();
    assert_eq!(tag_ids, vec![&tag]);
}

#[tokio::test]
async fn test_backup_import_rejects_transaction_with_unknown_tag_id() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let export_resp = client.get("/backup/export").await;
    let mut payload = export_resp.json();

    let created = client
        .post(
            "/transactions",
            txn(account_id, json!({"category_id": categories.id("Groceries"), "date": "2026-01-01"})),
        )
        .await;
    let created_id = created.json()["id"].clone();
    let items = client.get("/transactions").await.json()["items"].clone();
    // Reshape to the backup wire format (flat FKs, not nested account/category
    // objects) and inject a tag_id that doesn't exist in payload["tags"].
    let transactions: Vec<Value> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            json!({
                "id": t["id"],
                "account_id": t["account_id"],
                "category_id": t["category_id"],
                "transfer_account_id": t["transfer_account_id"],
                "type": t["type"],
                "amount": t["amount"],
                "description": t["description"],
                "merchant": t["merchant"],
                "notes": t["notes"],
                "date": t["date"],
                "tag_ids": if t["id"] == created_id { json!([999999]) } else { json!([]) },
            })
        })
        .collect();
    payload["transactions"] = Value::Array(transactions);

    let resp = client.post("/backup/import", payload).await;
    assert_eq!(resp.status, 400);
}

#[tokio::test]
async fn test_backup_roundtrip_preserves_transaction_splits() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let sweets = client
        .post("/categories", json!({"name": "Sweets", "kind": "expense", "color": "#7a869a", "parent_id": groceries}))
        .await
        .json()["id"]
        .clone();
    let created = client
        .post(
            "/transactions",
            txn(
                account_id,
                json!({
                    "amount": "100.00",
                    "category_id": null,
                    "splits": [
                        {"category_id": groceries, "amount": "70.00"},
                        {"category_id": sweets, "amount": "30.00", "note": "candy and snacks"},
                    ],
                }),
            ),
        )
        .await;
    let txn_id = created.json()["id"].clone();

    let payload = client.get("/backup/export").await.json();
    assert_eq!(payload["transaction_splits"].as_array().unwrap().len(), 2);

    let import_resp = client.post("/backup/import", payload).await;
    assert_eq!(import_resp.status, 200, "{}", import_resp.text);

    let listing = client.get("/transactions").await.json();
    let refetched = listing["items"].as_array().unwrap().iter().find(|t| t["id"] == txn_id).unwrap().clone();
    let splits = refetched["splits"].as_array().unwrap();
    let candy = splits.iter().find(|s| s["note"] == "candy and snacks").unwrap();
    assert_eq!(candy["category_id"], sweets);
    assert!(refetched["category"].is_null());
}
