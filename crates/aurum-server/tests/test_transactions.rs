//! Transactions: creation rules, filtering, pagination, sorting, and the
//! /transactions/years picker endpoint.
//!
//! These assert the actual contract the frontend relies on — e.g. the year/month
//! filter test below is the backend half of the "Recent Transactions showed last
//! month's data too" regression from Aug 2026 (see UPDATES.md): the bug was in
//! the frontend not passing year/month at all, but if the backend ever stopped
//! honoring those filters correctly, the frontend fix would silently stop
//! working and nothing would catch it except this test.

use std::collections::{HashMap, HashSet};

use chrono::Datelike;
use serde_json::{Value, json};

use crate::common::{Api, TestApp, dec, money, today, txn_payload as txn};

fn items(body: &Value) -> &Vec<Value> {
    body["items"].as_array().unwrap()
}

fn item_ids(body: &Value) -> Vec<Value> {
    items(body).iter().map(|item| item["id"].clone()).collect()
}

/// The 70/30 Groceries + `second` split receipt most split tests post.
fn split_payload(account_id: i64, groceries: i64, second: i64) -> Value {
    txn(
        account_id,
        json!({
            "amount": "100.00",
            "category_id": null,
            "splits": [
                {"category_id": groceries, "amount": "70.00"},
                {"category_id": second, "amount": "30.00"},
            ],
        }),
    )
}

#[tokio::test]
async fn test_bulk_create_transactions() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let resp = client
        .post(
            "/transactions/bulk",
            json!({
                "items": [
                    txn(account_id, json!({"category_id": groceries, "description": "csv row 1", "date": "2026-01-01"})),
                    txn(account_id, json!({"category_id": groceries, "description": "csv row 2", "date": "2026-01-02"})),
                ]
            }),
        )
        .await;
    assert_eq!(resp.status, 201);
    assert_eq!(resp.json()["created"], 2);

    let listed = client.get_with("/transactions", json!({"year": 2026, "month": 1})).await;
    assert_eq!(listed.json()["total"], 2);
}

#[tokio::test]
async fn test_bulk_create_is_all_or_nothing() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let salary = categories.id("Salary");
    let resp = client
        .post(
            "/transactions/bulk",
            json!({
                "items": [
                    txn(account_id, json!({"category_id": groceries, "description": "good row"})),
                    // An expense using an income category — rejected, and it
                    // should take the whole batch down with it.
                    txn(account_id, json!({"category_id": salary, "description": "bad row"})),
                ]
            }),
        )
        .await;
    assert_eq!(resp.status, 400);

    let listed = client.get("/transactions").await;
    assert_eq!(listed.json()["total"], 0);
}

#[tokio::test]
async fn test_create_expense_rejects_income_category() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let income_category = categories.get("Salary");
    let resp = client
        .post("/transactions", txn(account_id, json!({"type": "expense", "category_id": income_category["id"]})))
        .await;
    assert_eq!(resp.status, 400);
}

#[tokio::test]
async fn test_create_capitalizes_description_first_letter() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let category = categories.get("Groceries");
    let resp = client
        .post(
            "/transactions",
            txn(account_id, json!({"category_id": category["id"], "description": "траты на продукты"})),
        )
        .await;
    assert_eq!(resp.status, 201);
    assert_eq!(resp.json()["description"], "Траты на продукты");
}

#[tokio::test]
async fn test_update_capitalizes_description_first_letter() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let category = categories.get("Groceries");
    let created = client.post("/transactions", txn(account_id, json!({"category_id": category["id"]}))).await;
    let txn_id = created.json()["id"].clone();

    let resp = client.patch(&format!("/transactions/{txn_id}"), json!({"description": "new coffee shop"})).await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.json()["description"], "New coffee shop");
}

#[tokio::test]
async fn test_create_income_rejects_expense_category() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let expense_category = categories.get("Groceries");
    let resp = client
        .post(
            "/transactions",
            txn(account_id, json!({"type": "income", "amount": "500.00", "category_id": expense_category["id"]})),
        )
        .await;
    assert_eq!(resp.status, 400);
}

#[tokio::test]
async fn test_create_expense_with_matching_category_succeeds() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let category = categories.get("Groceries");
    let resp = client.post("/transactions", txn(account_id, json!({"category_id": category["id"]}))).await;
    assert_eq!(resp.status, 201);
    let body = resp.json();
    assert_eq!(body["category"]["id"], category["id"]);
    assert_eq!(money(&body["amount"]), dec("10.00"));
}

#[tokio::test]
async fn test_transfer_requires_destination_account() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let resp = client.post("/transactions", txn(account_id, json!({"type": "transfer", "amount": "50.00"}))).await;
    assert_eq!(resp.status, 422);
}

#[tokio::test]
async fn test_transfer_rejects_same_source_and_destination() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let resp = client
        .post(
            "/transactions",
            txn(account_id, json!({"type": "transfer", "amount": "50.00", "transfer_account_id": account_id})),
        )
        .await;
    assert_eq!(resp.status, 422);
}

#[tokio::test]
async fn test_transfer_rejects_a_category() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let other = client.post("/accounts", json!({"name": "Savings", "type": "savings", "currency": "USD"})).await;
    let other_id = other.json()["id"].clone();
    let resp = client
        .post(
            "/transactions",
            txn(
                account_id,
                json!({
                    "type": "transfer",
                    "amount": "50.00",
                    "transfer_account_id": other_id,
                    "category_id": categories.id("Groceries"),
                }),
            ),
        )
        .await;
    assert_eq!(resp.status, 422);
}

#[tokio::test]
async fn test_valid_transfer_between_two_accounts_succeeds() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let other = client.post("/accounts", json!({"name": "Savings", "type": "savings", "currency": "USD"})).await;
    let other_id = other.json()["id"].clone();
    let resp = client
        .post(
            "/transactions",
            txn(account_id, json!({"type": "transfer", "amount": "50.00", "transfer_account_id": other_id})),
        )
        .await;
    assert_eq!(resp.status, 201);
}

/// The exact shape of the Aug 2026 Dashboard bug: three transactions in
/// three different months must never leak into a query scoped to just one
/// of them.
#[tokio::test]
async fn test_list_filters_by_year_and_month_excludes_other_months() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let category_id = categories.id("Groceries");
    for (txn_date, description) in [
        ("2021-07-26", "july transaction"),
        ("2021-08-08", "august transaction"),
        ("2026-08-16", "current year august"),
    ] {
        let resp = client
            .post(
                "/transactions",
                txn(account_id, json!({"category_id": category_id, "date": txn_date, "description": description})),
            )
            .await;
        assert_eq!(resp.status, 201);
    }

    let resp = client.get_with("/transactions", json!({"year": 2021, "month": 8})).await;
    let body = resp.json();
    assert_eq!(body["total"], 1);
    let descriptions: Vec<&Value> = items(&body).iter().map(|item| &item["description"]).collect();
    assert_eq!(descriptions, vec!["August transaction"]);
}

#[tokio::test]
async fn test_list_filters_by_explicit_date_range() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let category_id = categories.id("Groceries");
    for txn_date in ["2024-01-01", "2024-06-15", "2024-12-31"] {
        client.post("/transactions", txn(account_id, json!({"category_id": category_id, "date": txn_date}))).await;
    }

    let resp = client.get_with("/transactions", json!({"start_date": "2024-02-01", "end_date": "2024-11-01"})).await;
    let body = resp.json();
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["date"], "2024-06-15");
}

/// The point of search is finding a purchase from an unknown month without
/// already knowing which month to filter by, so it must not be scoped by the
/// year/month params even when the caller happens to also pass them.
#[tokio::test]
async fn test_search_matches_description_across_any_period_ignoring_month_filter() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let category_id = categories.id("Groceries");
    for (txn_date, description) in
        [("2021-03-12", "PlayStation 5 console"), ("2024-11-02", "grocery run"), ("2026-08-16", "current month coffee")]
    {
        let resp = client
            .post(
                "/transactions",
                txn(account_id, json!({"category_id": category_id, "date": txn_date, "description": description})),
            )
            .await;
        assert_eq!(resp.status, 201);
    }

    let resp = client.get_with("/transactions", json!({"search": "playstation"})).await;
    let body = resp.json();
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["description"], "PlayStation 5 console");
}

#[tokio::test]
async fn test_search_matches_merchant_and_notes() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let category_id = categories.id("Groceries");
    client
        .post(
            "/transactions",
            txn(account_id, json!({"category_id": category_id, "description": "misc", "merchant": "Best Buy"})),
        )
        .await;
    client
        .post(
            "/transactions",
            txn(
                account_id,
                json!({"category_id": category_id, "description": "misc", "notes": "bought for a birthday gift"}),
            ),
        )
        .await;

    let merchant_resp = client.get_with("/transactions", json!({"search": "best buy"})).await;
    assert_eq!(merchant_resp.json()["total"], 1);

    let notes_resp = client.get_with("/transactions", json!({"search": "birthday"})).await;
    assert_eq!(notes_resp.json()["total"], 1);
}

#[tokio::test]
async fn test_search_with_no_matches_returns_empty() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let category_id = categories.id("Groceries");
    client
        .post("/transactions", txn(account_id, json!({"category_id": category_id, "description": "groceries"})))
        .await;

    let resp = client.get_with("/transactions", json!({"search": "nonexistent item xyz"})).await;
    let body = resp.json();
    assert_eq!(body["total"], 0);
    assert_eq!(body["items"], json!([]));
}

#[tokio::test]
async fn test_pagination_covers_every_item_with_no_overlap() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let category_id = categories.id("Groceries");
    for day in 1..26 {
        client
            .post(
                "/transactions",
                txn(
                    account_id,
                    json!({"category_id": category_id, "date": format!("2025-03-{day:02}"), "description": format!("txn-{day}")}),
                ),
            )
            .await;
    }

    let mut seen_ids: HashSet<i64> = HashSet::new();
    let mut page = 1;
    let mut total;
    loop {
        let resp = client.get_with("/transactions", json!({"page": page, "page_size": 10, "sort": "date_desc"})).await;
        let body = resp.json();
        total = body["total"].clone();
        if items(&body).is_empty() {
            break;
        }
        for item in items(&body) {
            let id = item["id"].as_i64().unwrap();
            assert!(!seen_ids.contains(&id), "pagination must not repeat an item across pages");
            seen_ids.insert(id);
        }
        page += 1;
    }

    assert_eq!(total, 25);
    assert_eq!(seen_ids.len(), 25);
}

#[tokio::test]
async fn test_sort_amount_desc_and_asc_are_opposite_orders() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let category_id = categories.id("Groceries");
    for amount in ["5.00", "50.00", "20.00"] {
        client.post("/transactions", txn(account_id, json!({"category_id": category_id, "amount": amount}))).await;
    }

    let desc = client.get_with("/transactions", json!({"sort": "amount_desc"})).await;
    let asc = client.get_with("/transactions", json!({"sort": "amount_asc"})).await;
    let desc_amounts: Vec<_> = items(&desc.json()).iter().map(|item| money(&item["amount"])).collect();
    let asc_amounts: Vec<_> = items(&asc.json()).iter().map(|item| money(&item["amount"])).collect();
    assert_eq!(desc_amounts, vec![dec("50.00"), dec("20.00"), dec("5.00")]);
    assert_eq!(asc_amounts, desc_amounts.into_iter().rev().collect::<Vec<_>>());
}

#[tokio::test]
async fn test_update_transaction_persists_changes() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let category_id = categories.id("Groceries");
    let created = client.post("/transactions", txn(account_id, json!({"category_id": category_id}))).await;
    let txn_id = created.json()["id"].clone();

    let resp =
        client.patch(&format!("/transactions/{txn_id}"), json!({"amount": "99.99", "description": "updated"})).await;
    assert_eq!(resp.status, 200);
    assert_eq!(money(&resp.json()["amount"]), dec("99.99"));
    assert_eq!(resp.json()["description"], "Updated");

    let refetched = client.get_with("/transactions", json!({"page_size": 1})).await;
    assert_eq!(money(&refetched.json()["items"][0]["amount"]), dec("99.99"));
}

#[tokio::test]
async fn test_update_transaction_rejects_category_kind_mismatch() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let created = client
        .post("/transactions", txn(account_id, json!({"type": "expense", "category_id": categories.id("Groceries")})))
        .await;
    let txn_id = created.json()["id"].clone();

    let resp = client.patch(&format!("/transactions/{txn_id}"), json!({"category_id": categories.id("Salary")})).await;
    assert_eq!(resp.status, 400);
}

#[tokio::test]
async fn test_delete_transaction_removes_it_from_listing() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let created =
        client.post("/transactions", txn(account_id, json!({"category_id": categories.id("Groceries")}))).await;
    let txn_id = created.json()["id"].clone();

    let delete_resp = client.delete(&format!("/transactions/{txn_id}")).await;
    assert_eq!(delete_resp.status, 204);

    let listing = client.get("/transactions").await;
    assert!(!item_ids(&listing.json()).contains(&txn_id));
}

#[tokio::test]
async fn test_years_endpoint_spans_earliest_transaction_through_current_year() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let category_id = categories.id("Groceries");
    client.post("/transactions", txn(account_id, json!({"category_id": category_id, "date": "2021-03-01"}))).await;
    client.post("/transactions", txn(account_id, json!({"category_id": category_id, "date": "2023-11-20"}))).await;

    let resp = client.get("/transactions/years").await;
    let current_year = today().year();
    assert_eq!(resp.json(), json!((2021..current_year + 1).collect::<Vec<_>>()));
}

#[tokio::test]
async fn test_years_endpoint_with_no_transactions_returns_only_current_year() {
    let client = TestApp::start().await;
    let resp = client.get("/transactions/years").await;
    assert_eq!(resp.json(), json!([today().year()]));
}

/// The create path rejects a transfer with no transfer_account_id; the
/// update path has to reject it too, or the row ends up moving money out of
/// an account and into nowhere — account_service subtracts the amount from
/// account_id but only credits transfer_account_id when it isn't NULL, so
/// the balance (and Net Worth's cash line, which walks the same rows) just
/// loses it.
#[tokio::test]
async fn test_update_cannot_turn_a_transaction_into_a_transfer_without_a_destination() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let created = client
        .post("/transactions", txn(account_id, json!({"amount": "40.00", "category_id": categories.id("Groceries")})))
        .await;
    let txn_id = created.json()["id"].clone();
    let balance_before = money(&client.get("/accounts").await.json()[0]["balance"]);

    let resp = client.patch(&format!("/transactions/{txn_id}"), json!({"type": "transfer", "category_id": null})).await;

    assert_eq!(resp.status, 400);
    assert_eq!(money(&client.get("/accounts").await.json()[0]["balance"]), balance_before);
}

/// Same rule as on create: a transfer to itself is a no-op row that still
/// shows up in the ledger as a transfer.
#[tokio::test]
async fn test_update_rejects_a_transfer_pointing_at_its_own_account() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let other = client.post("/accounts", json!({"name": "Savings", "type": "savings"})).await.json();
    let created = client
        .post(
            "/transactions",
            txn(account_id, json!({"type": "transfer", "transfer_account_id": other["id"], "category_id": null})),
        )
        .await;
    let txn_id = created.json()["id"].clone();

    let resp = client.patch(&format!("/transactions/{txn_id}"), json!({"transfer_account_id": account_id})).await;

    assert_eq!(resp.status, 400);
}

/// transfer_account_id on an expense is meaningless — balances only read
/// it for TRANSFER rows — and create already refuses it.
#[tokio::test]
async fn test_update_rejects_a_transfer_destination_on_a_non_transfer() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let other = client.post("/accounts", json!({"name": "Savings", "type": "savings"})).await.json();
    let created =
        client.post("/transactions", txn(account_id, json!({"category_id": categories.id("Groceries")}))).await;

    let resp = client
        .patch(&format!("/transactions/{}", created.json()["id"]), json!({"transfer_account_id": other["id"]}))
        .await;

    assert_eq!(resp.status, 400);
}

/// A categorized transfer would be counted as spending by the reports that
/// join on category — create rejects it, update must as well.
#[tokio::test]
async fn test_update_rejects_a_category_on_a_transfer() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let other = client.post("/accounts", json!({"name": "Savings", "type": "savings"})).await.json();
    let created = client
        .post(
            "/transactions",
            txn(account_id, json!({"type": "transfer", "transfer_account_id": other["id"], "category_id": null})),
        )
        .await;

    let resp = client
        .patch(&format!("/transactions/{}", created.json()["id"]), json!({"category_id": categories.id("Groceries")}))
        .await;

    assert_eq!(resp.status, 400);
}

/// Deleting an account nulls transfer_account_id on the transfers that
/// pointed at it (the FK is ON DELETE SET NULL), leaving a row that create
/// would reject. Refusing to *serialize* such a row takes the whole
/// transactions list down with a 500 — and with the list gone there is no
/// way left in the UI to delete the row and recover. Reading has to stay
/// possible; only writing is guarded.
#[tokio::test]
async fn test_a_transfer_whose_destination_account_was_deleted_can_still_be_listed() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let other = client.post("/accounts", json!({"name": "Savings", "type": "savings"})).await.json();
    let created = client
        .post(
            "/transactions",
            txn(account_id, json!({"type": "transfer", "transfer_account_id": other["id"], "category_id": null})),
        )
        .await;
    let txn_id = created.json()["id"].clone();
    assert_eq!(client.delete(&format!("/accounts/{}", other["id"])).await.status, 204);

    let listing = client.get("/transactions").await;

    assert_eq!(listing.status, 200);
    assert!(item_ids(&listing.json()).contains(&txn_id));
    assert_eq!(client.delete(&format!("/transactions/{txn_id}")).await.status, 204);
}

async fn subcategory(client: &Api, parent_id: i64, name: &str) -> i64 {
    let resp = client
        .post("/categories", json!({"name": name, "kind": "expense", "color": "#7a869a", "parent_id": parent_id}))
        .await;
    resp.json()["id"].as_i64().unwrap()
}

/// The hypermarket-receipt case: one purchase, part everyday groceries,
/// part sweets — a split's categories must share one parent (see
/// build_splits), so both lines here are Groceries itself and one of its
/// own subcategories. category_id stays empty on the row itself, and its
/// splits carry the categorization instead.
#[tokio::test]
async fn test_create_split_transaction_across_categories() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let sweets = subcategory(&client, groceries, "Sweets").await;
    let resp = client
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
    assert_eq!(resp.status, 201);
    let body = resp.json();
    assert!(body["category"].is_null());
    let splits: HashMap<String, Value> = body["splits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["category"]["name"].as_str().unwrap().to_string(), s.clone()))
        .collect();
    assert_eq!(money(&splits["Groceries"]["amount"]), dec("70.00"));
    assert_eq!(money(&splits["Sweets"]["amount"]), dec("30.00"));
    assert_eq!(splits["Sweets"]["note"], "candy and snacks");
}

/// The whole point of a split is dividing one purchase across the
/// subcategories of ONE parent — Groceries and Shopping are two unrelated
/// top-level categories, so mixing them must be rejected even though both
/// are individually valid expense categories.
#[tokio::test]
async fn test_create_split_rejects_categories_from_different_parents() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let shopping = categories.id("Shopping");
    let resp = client.post("/transactions", split_payload(account_id, groceries, shopping)).await;
    assert_eq!(resp.status, 400);
}

#[tokio::test]
async fn test_create_split_rejects_two_different_parents_subcategories() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let shopping = categories.id("Shopping");
    let sweets = subcategory(&client, groceries, "Sweets").await;
    let electronics = subcategory(&client, shopping, "Electronics").await;
    let resp = client
        .post(
            "/transactions",
            txn(
                account_id,
                json!({
                    "amount": "100.00",
                    "category_id": null,
                    "splits": [
                        {"category_id": sweets, "amount": "70.00"},
                        {"category_id": electronics, "amount": "30.00"},
                    ],
                }),
            ),
        )
        .await;
    assert_eq!(resp.status, 400);
}

#[tokio::test]
async fn test_create_split_rejects_category_id_alongside_splits() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let sweets = subcategory(&client, groceries, "Sweets").await;
    let resp = client
        .post(
            "/transactions",
            txn(
                account_id,
                json!({
                    "amount": "100.00",
                    "category_id": groceries,
                    "splits": [
                        {"category_id": groceries, "amount": "70.00"},
                        {"category_id": sweets, "amount": "30.00"},
                    ],
                }),
            ),
        )
        .await;
    assert_eq!(resp.status, 422);
}

#[tokio::test]
async fn test_create_split_rejects_a_single_split() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let resp = client
        .post(
            "/transactions",
            txn(
                account_id,
                json!({
                    "amount": "100.00",
                    "category_id": null,
                    "splits": [{"category_id": categories.id("Groceries"), "amount": "100.00"}],
                }),
            ),
        )
        .await;
    assert_eq!(resp.status, 422);
}

#[tokio::test]
async fn test_create_split_rejects_amounts_not_summing_to_total() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let sweets = subcategory(&client, groceries, "Sweets").await;
    let resp = client
        .post(
            "/transactions",
            txn(
                account_id,
                json!({
                    "amount": "100.00",
                    "category_id": null,
                    "splits": [
                        {"category_id": groceries, "amount": "70.00"},
                        {"category_id": sweets, "amount": "20.00"},
                    ],
                }),
            ),
        )
        .await;
    assert_eq!(resp.status, 422);
}

#[tokio::test]
async fn test_create_split_rejects_on_transfer() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let other = client.post("/accounts", json!({"name": "Savings", "type": "savings"})).await.json();
    let groceries = categories.id("Groceries");
    let sweets = subcategory(&client, groceries, "Sweets").await;
    let resp = client
        .post(
            "/transactions",
            txn(
                account_id,
                json!({
                    "type": "transfer",
                    "amount": "100.00",
                    "category_id": null,
                    "transfer_account_id": other["id"],
                    "splits": [
                        {"category_id": groceries, "amount": "70.00"},
                        {"category_id": sweets, "amount": "30.00"},
                    ],
                }),
            ),
        )
        .await;
    assert_eq!(resp.status, 422);
}

#[tokio::test]
async fn test_update_can_convert_a_transaction_into_a_split_and_back() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let sweets = subcategory(&client, groceries, "Sweets").await;
    let created =
        client.post("/transactions", txn(account_id, json!({"amount": "100.00", "category_id": groceries}))).await;
    let txn_id = created.json()["id"].clone();

    let split_resp = client
        .patch(
            &format!("/transactions/{txn_id}"),
            json!({
                "category_id": null,
                "splits": [
                    {"category_id": groceries, "amount": "70.00"},
                    {"category_id": sweets, "amount": "30.00"},
                ],
            }),
        )
        .await;
    assert_eq!(split_resp.status, 200);
    assert_eq!(split_resp.json()["splits"].as_array().unwrap().len(), 2);
    assert!(split_resp.json()["category"].is_null());

    let back_resp =
        client.patch(&format!("/transactions/{txn_id}"), json!({"category_id": groceries, "splits": []})).await;
    assert_eq!(back_resp.status, 200);
    assert_eq!(back_resp.json()["splits"], json!([]));
    assert_eq!(back_resp.json()["category"]["id"], groceries);
}

#[tokio::test]
async fn test_update_rejects_splits_not_summing_to_the_new_amount() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let sweets = subcategory(&client, groceries, "Sweets").await;
    let created = client.post("/transactions", split_payload(account_id, groceries, sweets)).await;
    let txn_id = created.json()["id"].clone();

    // Splits weren't touched by this patch, but the new amount no longer
    // matches their sum — the update must still be rejected.
    let resp = client.patch(&format!("/transactions/{txn_id}"), json!({"amount": "50.00"})).await;
    assert_eq!(resp.status, 400);
}

/// Same lesson as the transfer_account_id/deleted-account case: deleting
/// a category SET NULLs any split that pointed at it, and an edit to some
/// other field on the transaction must not be blocked by that pre-existing
/// state — the split's own category was never re-submitted, so nothing here
/// needs it to still exist.
#[tokio::test]
async fn test_update_of_unrelated_field_tolerates_a_split_whose_category_was_deleted() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let deletable = subcategory(&client, groceries, "Deletable").await;
    let created = client.post("/transactions", split_payload(account_id, groceries, deletable)).await;
    let txn_id = created.json()["id"].clone();
    assert_eq!(client.delete(&format!("/categories/{deletable}")).await.status, 204);

    let resp = client.patch(&format!("/transactions/{txn_id}"), json!({"description": "renamed"})).await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.json()["description"], "Renamed");
    assert!(resp.json()["splits"][1]["category"].is_null());
}

#[tokio::test]
async fn test_category_filter_matches_transactions_split_into_that_category() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let sweets = subcategory(&client, groceries, "Sweets").await;
    client.post("/transactions", split_payload(account_id, groceries, sweets)).await;

    let resp = client.get_with("/transactions", json!({"category_id": sweets})).await;
    assert_eq!(resp.json()["total"], 1);
}

#[tokio::test]
async fn test_delete_transaction_cascades_its_splits() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let sweets = subcategory(&client, groceries, "Sweets").await;
    let created = client.post("/transactions", split_payload(account_id, groceries, sweets)).await;
    let txn_id = created.json()["id"].clone();

    assert_eq!(client.delete(&format!("/transactions/{txn_id}")).await.status, 204);

    // If the split rows outlived their parent, this budget/dashboard-style
    // category filter would still find a (now-orphaned) match.
    let resp = client.get_with("/transactions", json!({"category_id": sweets})).await;
    assert_eq!(resp.json()["total"], 0);
}
