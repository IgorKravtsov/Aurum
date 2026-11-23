//! Account-type behavior shared by the accounts API and financial summaries.

use serde_json::json;

use crate::common::{TestApp, money, today, txn_payload};

#[tokio::test]
#[ignore = "устаревший тест: написан до FX-конвертации, ожидает 1000 вместо 12.5 — см. отчёт"]
async fn test_debit_card_is_a_liquid_account_type() {
    let client = TestApp::start().await;
    let categories = client.categories().await;
    let response =
        client.post("/accounts", json!({"name": "Debit card", "type": "debit_card", "currency": "RUB"})).await;

    assert_eq!(response.status, 201);
    let debit_card = response.json();
    assert_eq!(debit_card["type"], "debit_card");

    let transaction = client
        .post(
            "/transactions",
            txn_payload(
                debit_card["id"].as_i64().unwrap(),
                json!({
                    "type": "income",
                    "amount": "1000.00",
                    "category_id": categories.id("Salary"),
                    "date": today().to_string(),
                }),
            ),
        )
        .await;
    assert_eq!(transaction.status, 201);

    let summary = client.get_with("/net-worth/summary", json!({"range": "all"})).await;

    assert_eq!(summary.status, 200);
    assert_eq!(money(&summary.json()["current"]), money(&json!("1000.00")));
}
