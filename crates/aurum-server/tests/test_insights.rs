//! Proactive alerts (services/insights.rs): only the idle-cash
//! signal is covered here — the other three (negative cash flow, net worth
//! decline, over-budget) predate this test file and aren't touched by this
//! change.

use std::collections::HashSet;

use chrono::Duration;
use serde_json::json;

use crate::common::{Api, TestApp, today, txn_payload as txn};

fn stale_date() -> String {
    (today() - Duration::days(400)).to_string()
}

fn recent_date() -> String {
    today().to_string()
}

async fn alert_keys(client: &Api) -> HashSet<String> {
    let resp = client.get("/insights/alerts").await;
    resp.json()["alerts"].as_array().unwrap().iter().map(|alert| alert["key"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn test_idle_cash_flags_a_stale_large_balance() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    client
        .post(
            "/transactions",
            txn(
                account_id,
                json!({"type": "income", "amount": "5000.00", "category_id": categories.id("Salary"), "date": stale_date()}),
            ),
        )
        .await;

    assert!(alert_keys(&client).await.contains("idle_cash"));
}

#[tokio::test]
async fn test_idle_cash_ignores_a_recently_touched_account() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    client
        .post(
            "/transactions",
            txn(
                account_id,
                json!({"type": "income", "amount": "5000.00", "category_id": categories.id("Salary"), "date": stale_date()}),
            ),
        )
        .await;
    client
        .post(
            "/transactions",
            txn(
                account_id,
                json!({"type": "expense", "amount": "1.00", "category_id": categories.id("Groceries"), "date": recent_date()}),
            ),
        )
        .await;

    assert!(!alert_keys(&client).await.contains("idle_cash"));
}

#[tokio::test]
async fn test_idle_cash_ignores_investment_accounts() {
    let client = TestApp::start().await;
    let categories = client.categories().await;
    let investment =
        client.post("/accounts", json!({"name": "Brokerage", "type": "investment", "currency": "USD"})).await;
    let investment_id = investment.json()["id"].as_i64().unwrap();
    client
        .post(
            "/transactions",
            txn(
                investment_id,
                json!({"type": "income", "amount": "5000.00", "category_id": categories.id("Salary"), "date": stale_date()}),
            ),
        )
        .await;

    assert!(!alert_keys(&client).await.contains("idle_cash"));
}

#[tokio::test]
async fn test_idle_cash_includes_debit_cards() {
    let client = TestApp::start().await;
    let categories = client.categories().await;
    let debit_card =
        client.post("/accounts", json!({"name": "Debit card", "type": "debit_card", "currency": "USD"})).await;
    let debit_card_id = debit_card.json()["id"].as_i64().unwrap();
    client
        .post(
            "/transactions",
            txn(
                debit_card_id,
                json!({"type": "income", "amount": "5000.00", "category_id": categories.id("Salary"), "date": stale_date()}),
            ),
        )
        .await;

    assert!(alert_keys(&client).await.contains("idle_cash"));
}

#[tokio::test]
async fn test_idle_cash_respects_custom_threshold() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    client
        .post(
            "/transactions",
            txn(
                account_id,
                json!({"type": "income", "amount": "5000.00", "category_id": categories.id("Salary"), "date": stale_date()}),
            ),
        )
        .await;
    assert!(alert_keys(&client).await.contains("idle_cash"));

    let resp = client.patch("/settings", json!({"idle_cash_threshold_amount": "10000.00"})).await;
    assert_eq!(resp.status, 200);

    assert!(!alert_keys(&client).await.contains("idle_cash"));
}
