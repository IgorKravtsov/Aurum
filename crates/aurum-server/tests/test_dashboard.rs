//! Dashboard summary: the arithmetic and month-scoping behind the Overview
//! page's four stat cards and the spending-by-category donut.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::common::{TestApp, dec, money, txn_payload as txn};

/// `{row["name"]: row for row in rows}`.
fn by_name(rows: &Value) -> HashMap<String, Value> {
    rows.as_array().unwrap().iter().map(|row| (row["name"].as_str().unwrap().to_string(), row.clone())).collect()
}

#[tokio::test]
async fn test_summary_is_zero_for_a_month_with_no_transactions() {
    let client = TestApp::start().await;
    let resp = client.get_with("/dashboard/summary", json!({"year": 2030, "month": 1})).await;
    let body = resp.json();
    assert_eq!(money(&body["real_income"]), dec("0"));
    assert_eq!(money(&body["spent"]), dec("0"));
    assert_eq!(money(&body["net"]), dec("0"));
    assert_eq!(body["spending_by_category"], json!([]));
}

#[tokio::test]
async fn test_income_expense_and_net_only_count_the_selected_month() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let salary = categories.id("Salary");

    // In scope: August 2026.
    client
        .post(
            "/transactions",
            txn(
                account_id,
                json!({"type": "income", "amount": "3000.00", "category_id": salary, "date": "2026-08-01"}),
            ),
        )
        .await;
    client
        .post(
            "/transactions",
            txn(
                account_id,
                json!({"type": "expense", "amount": "1200.00", "category_id": groceries, "date": "2026-08-20"}),
            ),
        )
        .await;
    // Out of scope: same categories, different month — must not leak in.
    client
        .post(
            "/transactions",
            txn(
                account_id,
                json!({"type": "income", "amount": "9999.00", "category_id": salary, "date": "2026-07-31"}),
            ),
        )
        .await;
    client
        .post(
            "/transactions",
            txn(
                account_id,
                json!({"type": "expense", "amount": "9999.00", "category_id": groceries, "date": "2026-09-01"}),
            ),
        )
        .await;

    let resp = client.get_with("/dashboard/summary", json!({"year": 2026, "month": 8})).await;
    let body = resp.json();
    assert_eq!(money(&body["real_income"]), dec("3000.00"));
    assert_eq!(money(&body["spent"]), dec("1200.00"));
    assert_eq!(money(&body["net"]), dec("1800.00"));
}

#[tokio::test]
async fn test_transfers_are_excluded_from_income_and_spent() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let other = client.post("/accounts", json!({"name": "Savings", "type": "savings", "currency": "USD"})).await;
    let other_id = other.json()["id"].clone();
    client
        .post(
            "/transactions",
            txn(
                account_id,
                json!({"type": "transfer", "amount": "500.00", "transfer_account_id": other_id, "date": "2026-08-05"}),
            ),
        )
        .await;
    client
        .post(
            "/transactions",
            txn(
                account_id,
                json!({"type": "income", "amount": "100.00", "category_id": categories.id("Salary"), "date": "2026-08-05"}),
            ),
        )
        .await;

    let resp = client.get_with("/dashboard/summary", json!({"year": 2026, "month": 8})).await;
    let body = resp.json();
    assert_eq!(money(&body["real_income"]), dec("100.00"));
    assert_eq!(money(&body["spent"]), dec("0"));
    assert_eq!(money(&body["transferred_out"]), dec("500.00"));
}

#[tokio::test]
async fn test_spending_by_category_amounts_and_percent() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let dining = categories.id("Dining Out");
    client
        .post(
            "/transactions",
            txn(account_id, json!({"amount": "75.00", "category_id": groceries, "date": "2026-08-01"})),
        )
        .await;
    client
        .post("/transactions", txn(account_id, json!({"amount": "25.00", "category_id": dining, "date": "2026-08-02"})))
        .await;

    let resp = client.get_with("/dashboard/summary", json!({"year": 2026, "month": 8})).await;
    let breakdown = by_name(&resp.json()["spending_by_category"]);

    assert_eq!(money(&breakdown["Groceries"]["amount"]), dec("75.00"));
    assert_eq!(breakdown["Groceries"]["percent"].as_f64(), Some(75.0));
    assert_eq!(money(&breakdown["Dining Out"]["amount"]), dec("25.00"));
    assert_eq!(breakdown["Dining Out"]["percent"].as_f64(), Some(25.0));
}

#[tokio::test]
async fn test_subcategory_spending_rolls_up_into_its_parent() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let alcohol = client
        .post("/categories", json!({"name": "Alcohol", "kind": "expense", "color": "#e34948", "parent_id": groceries}))
        .await;
    let alcohol_id = alcohol.json()["id"].clone();

    client
        .post(
            "/transactions",
            txn(account_id, json!({"amount": "60.00", "category_id": groceries, "date": "2026-08-01"})),
        )
        .await;
    client
        .post(
            "/transactions",
            txn(account_id, json!({"amount": "15.00", "category_id": alcohol_id, "date": "2026-08-02"})),
        )
        .await;

    let resp = client.get_with("/dashboard/summary", json!({"year": 2026, "month": 8})).await;
    let breakdown = by_name(&resp.json()["spending_by_category"]);

    assert!(!breakdown.contains_key("Alcohol"));
    assert_eq!(money(&breakdown["Groceries"]["amount"]), dec("75.00"));
}

#[tokio::test]
async fn test_more_than_eight_expense_categories_roll_up_into_other() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let mut expense_categories: Vec<Value> =
        categories.values().iter().filter(|c| c["kind"] == "expense").cloned().collect();
    // The default seed only ships 8 expense categories (see db/seed.rs) —
    // add a 9th ourselves rather than depending on that number ever changing.
    let extra = client
        .post("/categories", json!({"name": "Extra Expense", "kind": "expense", "color": "#123456", "sort_order": 99}))
        .await;
    expense_categories.push(extra.json());
    assert_eq!(expense_categories.len(), 9);

    // Distinct amounts so we can tell which ones survive as their own slice.
    for (index, category) in expense_categories[..9].iter().enumerate() {
        client
            .post(
                "/transactions",
                txn(
                    account_id,
                    json!({"amount": format!("{}.00", (9 - index) * 10), "category_id": category["id"], "date": "2026-08-01"}),
                ),
            )
            .await;
    }

    let resp = client.get_with("/dashboard/summary", json!({"year": 2026, "month": 8})).await;
    let breakdown = resp.json()["spending_by_category"].as_array().unwrap().clone();

    assert_eq!(breakdown.len(), 9); // 8 explicit categories + one "Other" rollup
    let other: Vec<&Value> = breakdown.iter().filter(|row| row["category_id"].is_null()).collect();
    assert_eq!(other.len(), 1);
    assert_eq!(other[0]["name"], "Other");
    // The 9th (smallest, index 8) category's 10.00 must be folded into Other,
    // not silently dropped from the total.
    assert_eq!(money(&other[0]["amount"]), dec("10.00"));
}

/// A hypermarket receipt split between Groceries and one of its own
/// subcategories (Sweets) must show up as a single Groceries slice on the
/// donut with the full 100.00 total — a split's categories always share one
/// parent (see services/transactions.rs's build_splits), so it never
/// fragments into separate top-level slices — but the slice must expose a
/// children breakdown so the Groceries/Sweets split is still visible.
#[tokio::test]
async fn test_split_transaction_rolls_up_into_one_slice_with_a_children_breakdown() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    let sweets_resp = client
        .post("/categories", json!({"name": "Sweets", "kind": "expense", "color": "#7a869a", "parent_id": groceries}))
        .await;
    let sweets = sweets_resp.json()["id"].clone();
    client
        .post(
            "/transactions",
            txn(
                account_id,
                json!({
                    "amount": "100.00",
                    "category_id": null,
                    "date": "2026-08-01",
                    "splits": [
                        {"category_id": groceries, "amount": "70.00"},
                        {"category_id": sweets, "amount": "30.00"},
                    ],
                }),
            ),
        )
        .await;

    let resp = client.get_with("/dashboard/summary", json!({"year": 2026, "month": 8})).await;
    let body = resp.json();
    let breakdown = by_name(&body["spending_by_category"]);

    assert_eq!(money(&body["spent"]), dec("100.00"));
    assert!(!breakdown.contains_key("Sweets"));
    assert_eq!(money(&breakdown["Groceries"]["amount"]), dec("100.00"));

    let children = by_name(&breakdown["Groceries"]["children"]);
    // The 70.00 line was filed directly on Groceries itself (no
    // subcategory), so it shows up as a child entry with Groceries' own id
    // and name too — same as any genuine subcategory would.
    assert_eq!(money(&children["Groceries"]["amount"]), dec("70.00"));
    assert_eq!(money(&children["Sweets"]["amount"]), dec("30.00"));
}

/// A category funded by exactly one source (the common case — no
/// splitting, no subcategories involved) shouldn't show a redundant
/// one-item breakdown of itself.
#[tokio::test]
async fn test_a_single_category_slice_has_no_children_breakdown() {
    let client = TestApp::start().await;
    let account_id = client.account_id().await;
    let categories = client.categories().await;
    let groceries = categories.id("Groceries");
    client
        .post(
            "/transactions",
            txn(account_id, json!({"amount": "50.00", "category_id": groceries, "date": "2026-08-01"})),
        )
        .await;

    let resp = client.get_with("/dashboard/summary", json!({"year": 2026, "month": 8})).await;
    let breakdown = by_name(&resp.json()["spending_by_category"]);

    assert_eq!(breakdown["Groceries"]["children"], json!([]));
}
