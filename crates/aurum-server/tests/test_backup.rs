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

// A backup exported by a 1.x (Python) release must keep restoring after the
// 2.0.0 rewrite: only `aurum_backup_version` (the format) gates an import,
// `app_version` is informational. Pinned with a hand-written 1.3.2 file that
// predates tags, splits, crypto, budgets, goals and FX rates.
#[tokio::test]
async fn test_backup_from_1_3_2_restores_in_current_version() {
    let client = TestApp::start().await;
    let legacy = json!({
        "aurum_backup_version": 1,
        "exported_at": "2026-09-13T10:00:00.123456Z",
        "app_version": "1.3.2",
        "accounts": [
            {"id": 7, "name": "Legacy Account", "type": "checking", "currency": "PLN", "color": null, "is_archived": false},
        ],
        "categories": [
            {"id": 3, "name": "Legacy Category", "kind": "expense", "icon": null, "color": "#e34948", "sort_order": 0, "is_default": false},
        ],
        "transactions": [
            {
                "id": 11, "account_id": 7, "category_id": 3, "transfer_account_id": null, "type": "expense",
                "amount": "12.50", "description": "legacy", "merchant": null, "notes": null, "date": "2026-09-01",
            },
        ],
        "assets": [],
        "asset_valuations": [],
        "app_settings": {"currency": "PLN"},
    });

    let import_resp = client.post("/backup/import", legacy).await;
    assert_eq!(import_resp.status, 200, "{}", import_resp.text);
    assert_eq!(import_resp.json()["status"], "ok");

    let accounts = client.get("/accounts").await.json();
    assert!(accounts.as_array().unwrap().iter().any(|a| a["id"] == 7 && a["name"] == "Legacy Account"));
    let listing = client.get("/transactions").await.json();
    let restored = listing["items"].as_array().unwrap().iter().find(|t| t["id"] == 11).unwrap().clone();
    assert_eq!(restored["amount"], "12.50");

    // The next export is stamped with the running version, the format is unchanged.
    let exported = client.get("/backup/export").await.json();
    assert_eq!(exported["app_version"], aurum_domain::APP_VERSION);
    assert_eq!(exported["aurum_backup_version"], 1);
}

// Re-stamping a fresh export as coming from 1.3.2 changes nothing: the
// version string is never compared, only the format number.
#[tokio::test]
async fn test_backup_import_ignores_app_version() {
    let client = TestApp::start().await;
    for app_version in ["1.3.2", "1.0.0", "0.0.1", "3.7.1"] {
        let mut payload = client.get("/backup/export").await.json();
        payload["app_version"] = json!(app_version);
        let resp = client.post("/backup/import", payload).await;
        assert_eq!(resp.status, 200, "app_version {app_version}: {}", resp.text);
    }
}

// ... while a different *format* number is still refused.
#[tokio::test]
async fn test_backup_import_rejects_unsupported_format_version() {
    let client = TestApp::start().await;
    let mut payload = client.get("/backup/export").await.json();
    payload["aurum_backup_version"] = json!(2);
    let resp = client.post("/backup/import", payload).await;
    assert_eq!(resp.status, 400);
    assert!(resp.json()["detail"].as_str().unwrap().starts_with("Unsupported backup version 2"));
}
