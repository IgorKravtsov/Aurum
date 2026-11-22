//! Cash flow: month-by-month income vs. expense across an explicit date
//! range, or the full transaction history when no range is given.

use serde_json::json;

use crate::common::{TestApp, dec, money, txn_payload};

#[tokio::test]
async fn test_explicit_range_excludes_transactions_outside_it() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let _category_id = categories.id("Groceries");
    let salary_id = categories.id("Salary");
    for (amount, date) in [("1000.00", "2022-05-01"), ("9999.00", "2021-01-01"), ("9999.00", "2023-01-01")] {
        client
            .post(
                "/transactions",
                txn_payload(
                    account_id,
                    json!({"amount": amount, "type": "income", "category_id": salary_id, "date": date}),
                ),
            )
            .await;
    }

    let resp = client.get_with("/cash-flow", json!({"start_date": "2022-01-01", "end_date": "2022-12-31"})).await;
    let body = resp.json();
    assert_eq!(money(&body["total_income"]), dec("1000.00"));
    assert_eq!(body["points"].as_array().unwrap().len(), 12); // every month of 2022, including zero months
    assert_eq!(body["start_date"], "2022-01-01");
    assert_eq!(body["end_date"], "2022-12-31");
}

#[tokio::test]
async fn test_transfers_are_excluded_from_totals() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let other = client.post("/accounts", json!({"name": "Savings", "type": "savings", "currency": "USD"})).await;
    let other_id = other.json()["id"].clone();
    client
        .post(
            "/transactions",
            txn_payload(
                account_id,
                json!({"type": "transfer", "amount": "500.00", "transfer_account_id": other_id, "date": "2022-03-01"}),
            ),
        )
        .await;

    let resp = client.get_with("/cash-flow", json!({"start_date": "2022-01-01", "end_date": "2022-12-31"})).await;
    let body = resp.json();
    assert_eq!(money(&body["total_income"]), dec("0"));
    assert_eq!(money(&body["total_expense"]), dec("0"));
}

#[tokio::test]
async fn test_no_range_falls_back_to_earliest_and_latest_transaction_dates() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let category_id = categories.id("Groceries");
    client
        .post("/transactions", txn_payload(account_id, json!({"category_id": category_id, "date": "2021-06-15"})))
        .await;
    client
        .post("/transactions", txn_payload(account_id, json!({"category_id": category_id, "date": "2024-02-10"})))
        .await;

    let resp = client.get("/cash-flow").await;
    let body = resp.json();
    assert_eq!(body["start_date"], "2021-06-15");
    assert_eq!(body["end_date"], "2024-02-10");
}

#[tokio::test]
async fn test_no_transactions_returns_empty_points_and_null_bounds() {
    let client = TestApp::start().await;
    let resp = client.get("/cash-flow").await;
    let body = resp.json();
    assert_eq!(body["points"], json!([]));
    assert!(body["start_date"].is_null());
    assert!(body["end_date"].is_null());
    assert_eq!(money(&body["total_net"]), dec("0"));
}

#[tokio::test]
async fn test_monthly_points_split_income_and_expense_correctly() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let category_id = categories.id("Groceries");
    let salary_id = categories.id("Salary");
    client
        .post(
            "/transactions",
            txn_payload(
                account_id,
                json!({"type": "income", "amount": "2000.00", "category_id": salary_id, "date": "2022-04-01"}),
            ),
        )
        .await;
    client
        .post(
            "/transactions",
            txn_payload(
                account_id,
                json!({"type": "expense", "amount": "300.00", "category_id": category_id, "date": "2022-04-15"}),
            ),
        )
        .await;

    let resp = client.get_with("/cash-flow", json!({"start_date": "2022-04-01", "end_date": "2022-04-30"})).await;
    let point = resp.json()["points"][0].clone();
    assert_eq!(point["year"], 2022);
    assert_eq!(point["month"], 4);
    assert_eq!(money(&point["income"]), dec("2000.00"));
    assert_eq!(money(&point["expense"]), dec("300.00"));
    assert_eq!(money(&point["net"]), dec("1700.00"));
}
