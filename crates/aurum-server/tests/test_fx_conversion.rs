//! Mixed-currency amounts are converted before any cross-account aggregate.
//!
//! NBP itself is never called: the app is pointed at a local fake NBP
//! (common/fakes.rs). Two tests here drive `services::fx::convert_amounts`
//! directly (there's no endpoint that exposes it); in the pytest suite they
//! were `python_internal` and monkeypatched the NBP fetch — here the fake NBP
//! plays that part (an outage, or a refreshed rate).

use std::collections::HashMap;

use aurum_server::services::fx::{self, MoneyAmount};
use chrono::{Duration, NaiveDate, Utc};
use serde_json::json;

use crate::common::{TestApp, dec, money, today, txn_payload};

#[tokio::test]
async fn test_mixed_currency_aggregates_use_historical_nbp_rates() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    // Constant test rates: NBP publishes 1 USD = 4 PLN and 1 UAH = 0.1 PLN
    // on every business day; any other currency is unknown to it.
    client.nbp.serve_rates(&[("USD", "4"), ("UAH", "0.1")]);
    client.patch("/settings", json!({"currency": "PLN"})).await;
    let usd_account = client.patch(&format!("/accounts/{account_id}"), json!({"currency": "USD"})).await.json();
    let uah_account =
        client.post("/accounts", json!({"name": "UAH card", "type": "checking", "currency": "UAH"})).await.json();
    let pln_account =
        client.post("/accounts", json!({"name": "PLN account", "type": "checking", "currency": "PLN"})).await.json();

    sqlx::query(
        "INSERT INTO fx_rates (currency, requested_date, effective_date, rate_to_pln, provider) \
         VALUES ($1, $3, $3, $4, 'test'), ($2, $3, $3, $5, 'test')",
    )
    .bind("USD")
    .bind("UAH")
    .bind(NaiveDate::from_ymd_opt(2026, 1, 15).unwrap())
    .bind(dec("4.000000000000000000"))
    .bind(dec("0.100000000000000000"))
    .execute(client.pool())
    .await
    .unwrap();

    let usd_id = usd_account["id"].as_i64().unwrap();
    let uah_id = uah_account["id"].as_i64().unwrap();
    let pln_id = pln_account["id"].as_i64().unwrap();
    assert_eq!(
        client
            .post(
                "/transactions",
                txn_payload(
                    usd_id,
                    json!({"type": "income", "amount": "100.00", "category_id": categories.id("Salary")})
                ),
            )
            .await
            .status,
        201
    );
    assert_eq!(
        client
            .post(
                "/transactions",
                txn_payload(uah_id, json!({"amount": "1000.00", "category_id": categories.id("Groceries")}))
            )
            .await
            .status,
        201
    );
    assert_eq!(
        client
            .post(
                "/transactions",
                txn_payload(pln_id, json!({"amount": "50.00", "category_id": categories.id("Groceries")}))
            )
            .await
            .status,
        201
    );
    let mut transfer_payload = txn_payload(
        usd_id,
        json!({"type": "transfer", "amount": "10.00", "category_id": null, "transfer_account_id": uah_id}),
    );
    let missing_destination = client.post("/transactions", transfer_payload.clone()).await;
    assert_eq!(missing_destination.status, 400);
    transfer_payload["destination_amount"] = json!("390.00");
    let transfer = client.post("/transactions", transfer_payload).await;
    assert_eq!(transfer.status, 201);
    assert_eq!(money(&transfer.json()["destination_amount"]), dec("390"));

    let dashboard = client.get_with("/dashboard/summary", json!({"year": 2026, "month": 1})).await.json();
    assert_eq!(money(&dashboard["real_income"]), dec("400"));
    assert_eq!(money(&dashboard["spent"]), dec("150"));
    assert_eq!(money(&dashboard["net"]), dec("250"));
    assert_eq!(money(&dashboard["spending_by_category"][0]["amount"]), dec("150"));

    let cash_flow =
        client.get_with("/cash-flow", json!({"start_date": "2026-01-01", "end_date": "2026-01-31"})).await.json();
    assert_eq!(money(&cash_flow["total_income"]), dec("400"));
    assert_eq!(money(&cash_flow["total_expense"]), dec("150"));

    client.post("/budgets", json!({"category_id": categories.id("Groceries"), "monthly_limit": "200.00"})).await;
    let budget = client.get_with("/budgets/status", json!({"year": 2026, "month": 1})).await.json()["items"][0].clone();
    assert_eq!(money(&budget["spent"]), dec("150"));
    assert_eq!(budget["percent"].as_f64(), Some(75.0));

    let report = client
        .get_with(
            "/reports/category-spending",
            json!({"category_id": categories.id("Groceries"), "start_date": "2026-01-01", "end_date": "2026-01-31"}),
        )
        .await
        .json();
    assert_eq!(money(&report["total_amount"]), dec("150"));

    let accounts: HashMap<String, serde_json::Value> = client
        .get("/accounts")
        .await
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|item| (item["name"].as_str().unwrap().to_string(), item.clone()))
        .collect();
    assert_eq!(money(&accounts["Main Account"]["balance"]), dec("90"));
    assert_eq!(money(&accounts["UAH card"]["balance"]), dec("-610"));
    assert_eq!(money(&accounts["PLN account"]["balance"]), dec("-50"));

    let net_worth_response = client.get_with("/net-worth/summary", json!({"range": "30d"})).await;
    assert_eq!(net_worth_response.status, 200, "{}", net_worth_response.text);
    let net_worth = net_worth_response.json();
    assert_eq!(money(&net_worth["current"]), dec("249"));
    let cash = net_worth["breakdown"].as_array().unwrap().iter().find(|item| item["key"] == "cash").unwrap();
    assert_eq!(money(&cash["amount"]), dec("249"));

    let currency_change = client.patch("/settings", json!({"currency": "USD"})).await;
    assert_eq!(currency_change.status, 409);
}

/// Was `python_internal` (monkeypatched `_fetch_nbp_currency` to raise a
/// 503): here the fake NBP is down for the whole test instead.
#[tokio::test]
async fn test_cached_weekend_rate_survives_backup_without_provider() {
    let client = TestApp::start().await;
    let saturday = NaiveDate::from_ymd_opt(2026, 1, 17).unwrap();
    let friday = NaiveDate::from_ymd_opt(2026, 1, 16).unwrap();
    sqlx::query(
        "INSERT INTO fx_rates (currency, requested_date, effective_date, rate_to_pln, provider) \
         VALUES ('UAH', $1, $1, $2, 'test')",
    )
    .bind(friday)
    .bind(dec("0.083000000000000000"))
    .execute(client.pool())
    .await
    .unwrap();

    // provider unavailable
    client.nbp.fail(503);
    let converted =
        fx::convert_amounts(&client.state, &[MoneyAmount::new("weekend", dec("1000"), "UAH", saturday)], "PLN")
            .await
            .expect("a cached nearby publication covers the outage");

    assert_eq!(converted["weekend"], dec("83"));

    let backup = client.get("/backup/export").await.json();
    assert_eq!(backup["fx_rates"][0]["effective_date"], "2026-01-16");
    assert_eq!(client.post("/backup/import", backup).await.status, 200);
    let restored =
        fx::convert_amounts(&client.state, &[MoneyAmount::new("restored", dec("1000"), "UAH", saturday)], "PLN")
            .await
            .expect("the restored cache still covers the outage");
    assert_eq!(restored["restored"], dec("83"));
}

/// Was `python_internal` (monkeypatched `_fetch_nbp_currency` to assert it
/// is asked for exactly USD on today and to answer 5): here the fake NBP
/// publishes USD = 5 and records what it was asked for.
#[tokio::test]
async fn test_expired_provisional_rate_is_refreshed() {
    let client = TestApp::start().await;
    let today = today();
    sqlx::query(
        "INSERT INTO fx_rates (currency, requested_date, effective_date, rate_to_pln, provider, fetched_at) \
         VALUES ('USD', $1, $2, $3, 'test', $4)",
    )
    .bind(today)
    .bind(today - Duration::days(1))
    .bind(dec("4"))
    .bind(Utc::now() - Duration::hours(2))
    .execute(client.pool())
    .await
    .unwrap();

    client.nbp.serve_rates(&[("USD", "5")]);
    let converted = fx::convert_amounts(&client.state, &[MoneyAmount::new("today", dec("2"), "USD", today)], "PLN")
        .await
        .expect("refreshed rate");

    // currency == "USD" and requested_dates == {today}: every NBP call was
    // for USD, over a window ending on today.
    let requests = client.nbp.requests();
    assert!(!requests.is_empty());
    for request in &requests {
        assert!(request.starts_with("/a/usd/"), "unexpected NBP request {request}");
        assert!(request.ends_with(&format!("/{today}/")), "unexpected NBP request {request}");
    }
    assert_eq!(converted["today"], dec("10"));
}

#[tokio::test]
async fn test_recurring_transfer_locks_both_account_currencies() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let destination = client
        .post("/accounts", json!({"name": "Recurring destination", "type": "savings", "currency": "USD"}))
        .await
        .json();
    let rule = client
        .post(
            "/recurring",
            json!({
                "account_id": account_id,
                "category_id": null,
                "transfer_account_id": destination["id"],
                "type": "transfer",
                "amount": "25.00",
                "description": "Monthly transfer",
                "frequency": "monthly",
                "anchor_date": "2026-01-15",
            }),
        )
        .await;
    assert_eq!(rule.status, 201);
    assert_eq!(money(&rule.json()["destination_amount"]), dec("25"));

    let response = client.patch(&format!("/accounts/{}", destination["id"]), json!({"currency": "UAH"})).await;

    assert_eq!(response.status, 409);
}

#[tokio::test]
async fn test_base_currency_is_immutable_after_transaction_history() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    client.post("/transactions", txn_payload(account_id, json!({"category_id": categories.id("Groceries")}))).await;

    let response = client.patch("/settings", json!({"currency": "PLN"})).await;

    assert_eq!(response.status, 409);
}

#[tokio::test]
async fn test_account_currency_is_immutable_after_history() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    client.post("/transactions", txn_payload(account_id, json!({"category_id": categories.id("Groceries")}))).await;

    let response = client.patch(&format!("/accounts/{account_id}"), json!({"currency": "PLN"})).await;

    assert_eq!(response.status, 409);
    assert!(response.json()["detail"].as_str().unwrap().contains("cannot be changed"));
}
