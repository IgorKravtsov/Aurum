//! Crypto holdings: buy/sell transaction log, weighted-average cost basis,
//! and the lazy/forced price sync. CoinGecko itself is never called in tests —
//! the app is pointed at a local fake CoinGecko (common/fakes.rs)
//! serving a canned market-data feed instead, same way any external dependency
//! would be.

use serde_json::{Value, json};

use crate::common::{Api, MarketPoint, TestApp, dec, money};

fn point(price: &str, change_1h: Option<&str>, change_24h: Option<&str>, change_7d: Option<&str>) -> MarketPoint {
    MarketPoint {
        price: dec(price),
        change_1h: change_1h.map(dec),
        change_24h: change_24h.map(dec),
        change_7d: change_7d.map(dec),
        ..MarketPoint::default()
    }
}

/// `_point(price)` — just a price, no change percentages.
fn price(price: &str) -> MarketPoint {
    point(price, None, None, None)
}

async fn add_bitcoin_with(
    client: &Api,
    quantity: &str,
    price_per_unit: &str,
    risk_level: Option<&str>,
    network: Option<&str>,
) -> Value {
    let mut payload = json!({
        "coingecko_id": "bitcoin",
        "symbol": "btc",
        "name": "Bitcoin",
        "quantity": quantity,
        "price_per_unit": price_per_unit,
        "date": "2026-01-01",
    });
    if let Some(risk_level) = risk_level {
        payload["risk_level"] = json!(risk_level);
    }
    if let Some(network) = network {
        payload["network"] = json!(network);
    }
    let resp = client.post("/crypto/holdings", payload).await;
    assert_eq!(resp.status, 201, "{}", resp.text);
    resp.json()
}

/// `_add_bitcoin(client, quantity="0.5", price_per_unit="40000")`.
async fn add_bitcoin(client: &Api, quantity: &str, price_per_unit: &str) -> Value {
    add_bitcoin_with(client, quantity, price_per_unit, None, None).await
}

/// `_add_bitcoin(client)` with the defaults.
async fn add_default_bitcoin(client: &Api) -> Value {
    add_bitcoin(client, "0.5", "40000").await
}

fn holdings(body: &Value) -> &Vec<Value> {
    body["holdings"].as_array().unwrap()
}

/// POST /crypto/holdings for `coingecko_id` in a given portfolio.
async fn add_bitcoin_to_portfolio(client: &Api, portfolio_id: &Value, quantity: &str) -> Value {
    let resp = client
        .post(
            "/crypto/holdings",
            json!({
                "portfolio_id": portfolio_id,
                "coingecko_id": "bitcoin",
                "symbol": "btc",
                "name": "Bitcoin",
                "quantity": quantity,
                "price_per_unit": "40000",
                "date": "2026-01-01",
            }),
        )
        .await;
    assert_eq!(resp.status, 201, "{}", resp.text);
    resp.json()
}

#[tokio::test]
async fn test_create_holding_seeds_todays_value_and_position_immediately() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);

    let holding = add_bitcoin(&client, "0.5", "40000").await;

    assert_eq!(holding["symbol"], "BTC"); // normalized to uppercase
    assert_eq!(money(&holding["quantity"]), dec("0.5"));
    assert_eq!(money(&holding["avg_buy_price"]), dec("40000"));
    assert_eq!(money(&holding["current_price"]), dec("50000"));
    assert_eq!(money(&holding["value"]), dec("25000")); // 0.5 * 50000
    assert_eq!(money(&holding["cost_basis"]), dec("20000")); // 0.5 * 40000
    assert_eq!(money(&holding["profit_loss"]), dec("5000"));
    let profit_loss_percent = holding["profit_loss_percent"].as_f64().unwrap();
    assert_eq!((profit_loss_percent * 100.0).round() / 100.0, 25.0);
}

#[tokio::test]
async fn test_holding_shows_up_in_net_worth_breakdown() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    add_bitcoin(&client, "1", "40000").await;

    let resp = client.get("/net-worth/summary").await;

    let body = resp.json();
    let crypto = body["breakdown"].as_array().unwrap().iter().find(|item| item["key"] == "crypto").unwrap();
    assert_eq!(money(&crypto["amount"]), dec("50000"));
}

#[tokio::test]
async fn test_second_buy_blends_into_weighted_average_cost() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    let holding = add_bitcoin(&client, "1", "40000").await; // cost basis: 40000

    client.coingecko.forbid("adding a transaction must not call CoinGecko");

    let resp = client
        .post(
            &format!("/crypto/holdings/{}/transactions", holding["asset_id"]),
            json!({"type": "buy", "quantity": "1", "price_per_unit": "60000", "date": "2026-02-01"}),
        )
        .await;

    assert_eq!(resp.status, 201, "{}", resp.text);
    let body = resp.json();
    assert_eq!(money(&body["quantity"]), dec("2"));
    // (1*40000 + 1*60000) / 2 = 50000
    assert_eq!(money(&body["avg_buy_price"]), dec("50000"));
    assert_eq!(money(&body["value"]), dec("100000")); // 2 * 50000 (last known price, unchanged)
}

#[tokio::test]
async fn test_sell_reduces_quantity_but_not_the_average_cost_of_what_remains() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    let holding = add_bitcoin(&client, "2", "40000").await; // avg cost 40000

    let resp = client
        .post(
            &format!("/crypto/holdings/{}/transactions", holding["asset_id"]),
            json!({"type": "sell", "quantity": "1", "price_per_unit": "55000", "date": "2026-02-01"}),
        )
        .await;

    assert_eq!(resp.status, 201, "{}", resp.text);
    let body = resp.json();
    assert_eq!(money(&body["quantity"]), dec("1"));
    assert_eq!(money(&body["avg_buy_price"]), dec("40000")); // unchanged by the sell
    assert_eq!(money(&body["value"]), dec("50000")); // 1 * 50000
}

#[tokio::test]
async fn test_selling_more_than_held_is_rejected() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    let holding = add_bitcoin(&client, "1", "40000").await;

    let resp = client
        .post(
            &format!("/crypto/holdings/{}/transactions", holding["asset_id"]),
            json!({"type": "sell", "quantity": "2", "price_per_unit": "55000", "date": "2026-02-01"}),
        )
        .await;

    assert_eq!(resp.status, 400);
}

#[tokio::test]
async fn test_deleting_a_transaction_recomputes_the_position() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    let holding = add_bitcoin(&client, "1", "40000").await;

    let second = client
        .post(
            &format!("/crypto/holdings/{}/transactions", holding["asset_id"]),
            json!({"type": "buy", "quantity": "1", "price_per_unit": "60000", "date": "2026-02-01"}),
        )
        .await;
    let transactions = client.get(&format!("/crypto/holdings/{}/transactions", holding["asset_id"])).await.json();
    let tx_id = transactions
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["price_per_unit"] == "60000.000000000000000000")
        .map(|t| t["id"].clone())
        .unwrap();
    assert_eq!(second.json()["quantity"], "2.000000000000000000");

    let resp = client.delete(&format!("/crypto/transactions/{tx_id}")).await;
    assert_eq!(resp.status, 204);

    let refreshed = holdings(&client.get("/crypto/holdings").await.json())[0].clone();
    assert_eq!(money(&refreshed["quantity"]), dec("1"));
    assert_eq!(money(&refreshed["avg_buy_price"]), dec("40000"));
}

#[tokio::test]
async fn test_transaction_history_lists_newest_first() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    let holding = add_bitcoin(&client, "1", "40000").await;
    client
        .post(
            &format!("/crypto/holdings/{}/transactions", holding["asset_id"]),
            json!({"type": "buy", "quantity": "1", "price_per_unit": "60000", "date": "2026-02-01"}),
        )
        .await;

    let resp = client.get(&format!("/crypto/holdings/{}/transactions", holding["asset_id"])).await;

    assert_eq!(resp.status, 200);
    let dates: Vec<Value> = resp.json().as_array().unwrap().iter().map(|t| t["date"].clone()).collect();
    assert_eq!(dates, vec!["2026-02-01", "2026-01-01"]);
}

#[tokio::test]
async fn test_deleting_the_asset_removes_the_crypto_holding_too() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    let holding = add_default_bitcoin(&client).await;

    let resp = client.delete(&format!("/assets/{}", holding["asset_id"])).await;
    assert_eq!(resp.status, 204);

    let resp = client.get("/crypto/holdings").await;
    assert_eq!(resp.json()["holdings"], json!([]));
}

#[tokio::test]
async fn test_get_holdings_does_not_resync_within_24_hours() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    add_default_bitcoin(&client).await;

    // Each /coins/markets call's ids land in `market_calls`; the feed is empty.
    client.coingecko.record_market_calls();
    client.coingecko.serve_markets(&[]);

    let resp = client.get("/crypto/holdings").await;

    assert_eq!(resp.json()["synced"], json!(false));
    assert_eq!(client.coingecko.market_calls(), Vec::<Vec<String>>::new()); // creation's own sync already satisfied the 24h window
}

#[tokio::test]
async fn test_manual_refresh_button_bypasses_the_24h_window_and_updates_changes() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    add_bitcoin(&client, "0.5", "40000").await;

    client.coingecko.record_market_calls();
    client.coingecko.serve_markets(&[("bitcoin", point("60000", Some("0.5"), Some("2.1"), Some("-3.4")))]);

    let resp = client.post_empty("/crypto/refresh").await;

    assert_eq!(resp.json()["synced"], json!(true));
    assert_eq!(client.coingecko.market_calls(), vec![vec!["bitcoin".to_string()]]);
    let holding = holdings(&resp.json())[0].clone();
    assert_eq!(money(&holding["value"]), dec("30000")); // 0.5 * 60000
    assert_eq!(money(&holding["price_change_1h"]), dec("0.5"));
    assert_eq!(money(&holding["price_change_24h"]), dec("2.1"));
    assert_eq!(money(&holding["price_change_7d"]), dec("-3.4"));
}

#[tokio::test]
async fn test_coingecko_outage_keeps_last_known_value_instead_of_failing() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    add_bitcoin(&client, "0.5", "40000").await;

    // Simulated outage: CoinGecko answers 503 to everything.
    client.coingecko.fail(503);

    let resp = client.post_empty("/crypto/refresh").await;

    assert_eq!(resp.status, 200);
    let body = resp.json();
    assert_eq!(body["synced"], json!(false));
    assert_eq!(body["error_key"], "unreachable");
    assert_eq!(money(&holdings(&body)[0]["value"]), dec("25000")); // unchanged from creation
}

/// Was `@pytest.mark.server_env(AURUM_COINGECKO_API_KEY="")`.
#[tokio::test]
async fn test_create_rejects_when_no_api_key_configured() {
    let client = TestApp::start_with(|settings| settings.coingecko_api_key = String::new()).await;

    let resp = client
        .post(
            "/crypto/holdings",
            json!({
                "coingecko_id": "bitcoin",
                "symbol": "btc",
                "name": "Bitcoin",
                "quantity": "0.5",
                "price_per_unit": "40000",
                "date": "2026-01-01",
            }),
        )
        .await;

    assert_eq!(resp.status, 400);
}

/// A crypto holding is an Asset underneath, so it would still show up
/// after a restore even without this — but as an inert manual asset,
/// having silently lost which coin it was and its whole buy/sell history
/// (so quantity and avg buy price couldn't be recomputed). Backup must
/// carry both CryptoHolding and CryptoTransaction rows.
#[tokio::test]
async fn test_backup_roundtrip_preserves_holding_and_transaction_log() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    let holding = add_bitcoin(&client, "1", "40000").await;
    client
        .post(
            &format!("/crypto/holdings/{}/transactions", holding["asset_id"]),
            json!({"type": "buy", "quantity": "1", "price_per_unit": "60000", "date": "2026-02-01"}),
        )
        .await;

    let payload = client.get("/backup/export").await.json();
    assert_eq!(payload["crypto_holdings"].as_array().unwrap().len(), 1);
    assert_eq!(payload["crypto_transactions"].as_array().unwrap().len(), 2);

    let import_resp = client.post("/backup/import", payload).await;
    assert_eq!(import_resp.status, 200, "{}", import_resp.text);

    let restored = holdings(&client.get("/crypto/holdings").await.json())[0].clone();
    assert_eq!(restored["coingecko_id"], "bitcoin");
    assert_eq!(restored["asset_id"], holding["asset_id"]);
    assert_eq!(money(&restored["quantity"]), dec("2"));
    assert_eq!(money(&restored["avg_buy_price"]), dec("50000"));
    assert_eq!(money(&restored["value"]), dec("100000")); // last_price survived too (2 * 50000)
}

#[tokio::test]
async fn test_backup_roundtrip_preserves_portfolio_assignment() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    let portfolio = client.post("/crypto/portfolios", json!({"name": "Long-term"})).await.json();
    add_bitcoin_to_portfolio(&client, &portfolio["id"], "1").await;

    let payload = client.get("/backup/export").await.json();
    assert_eq!(payload["crypto_portfolios"].as_array().unwrap().len(), 1);
    assert_eq!(payload["crypto_holdings"][0]["portfolio_id"], portfolio["id"]);

    let import_resp = client.post("/backup/import", payload).await;
    assert_eq!(import_resp.status, 200, "{}", import_resp.text);

    let restored_portfolios = client.get("/crypto/portfolios").await.json();
    assert_eq!(restored_portfolios.as_array().unwrap().len(), 1);
    assert_eq!(restored_portfolios[0]["name"], "Long-term");
    let restored_holding = holdings(&client.get("/crypto/holdings").await.json())[0].clone();
    assert_eq!(restored_holding["portfolio_id"], restored_portfolios[0]["id"]);
}

/// A backup exported before crypto portfolios existed has no
/// crypto_portfolios list and every holding's portfolio_id is absent —
/// restore_backup() must still produce a valid NOT NULL portfolio_id
/// instead of failing the import.
#[tokio::test]
async fn test_restoring_a_pre_portfolios_backup_falls_back_to_a_default_portfolio() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    let holding = add_bitcoin(&client, "1", "40000").await;

    let mut payload = client.get("/backup/export").await.json();
    payload["crypto_portfolios"] = json!([]);
    payload["crypto_holdings"][0]["portfolio_id"] = Value::Null;

    let import_resp = client.post("/backup/import", payload).await;
    assert_eq!(import_resp.status, 200, "{}", import_resp.text);

    let restored_portfolios = client.get("/crypto/portfolios").await.json();
    assert_eq!(restored_portfolios.as_array().unwrap().len(), 1);
    let restored_holding = holdings(&client.get("/crypto/holdings").await.json())[0].clone();
    assert_eq!(restored_holding["asset_id"], holding["asset_id"]);
    assert_eq!(restored_holding["portfolio_id"], restored_portfolios[0]["id"]);
}

#[tokio::test]
async fn test_history_reflects_current_total_crypto_value() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    add_bitcoin(&client, "1", "40000").await;

    let resp = client.get("/crypto/history?range=30d").await;

    assert_eq!(resp.status, 200);
    let body = resp.json();
    let series = body["series"].as_array().unwrap();
    assert_eq!(money(&body["current"]), dec("50000"));
    assert_eq!(series.len(), 30);
    assert_eq!(money(&series[series.len() - 1]["value"]), dec("50000"));
    assert_eq!(money(&series[0]["value"]), dec("0")); // nothing existed 30 days ago
}

#[tokio::test]
async fn test_history_is_empty_series_with_no_holdings() {
    let client = TestApp::start().await;
    let resp = client.get("/crypto/history?range=7d").await;

    assert_eq!(resp.status, 200);
    let body = resp.json();
    assert_eq!(body["series"], json!([]));
    assert_eq!(money(&body["current"]), dec("0"));
}

#[tokio::test]
async fn test_history_rejects_unknown_range() {
    let client = TestApp::start().await;
    let resp = client.get("/crypto/history?range=24h").await;

    assert_eq!(resp.status, 422);
}

#[tokio::test]
async fn test_update_transaction_recomputes_position_without_calling_coingecko() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    let holding = add_bitcoin(&client, "1", "40000").await;
    let tx_id =
        client.get(&format!("/crypto/holdings/{}/transactions", holding["asset_id"])).await.json()[0]["id"].clone();

    client.coingecko.forbid("editing a transaction must not call CoinGecko");

    let resp = client.patch(&format!("/crypto/transactions/{tx_id}"), json!({"quantity": "2"})).await;

    assert_eq!(resp.status, 200, "{}", resp.text);
    let body = resp.json();
    assert_eq!(money(&body["quantity"]), dec("2"));
    assert_eq!(money(&body["value"]), dec("100000")); // 2 * 50000 (last known price)
}

#[tokio::test]
async fn test_update_transaction_edits_are_reflected_in_history() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    let holding = add_bitcoin(&client, "1", "40000").await;
    let tx_id =
        client.get(&format!("/crypto/holdings/{}/transactions", holding["asset_id"])).await.json()[0]["id"].clone();

    let resp = client
        .patch(&format!("/crypto/transactions/{tx_id}"), json!({"price_per_unit": "45000", "note": "typo fix"}))
        .await;
    assert_eq!(resp.status, 200, "{}", resp.text);

    let transactions = client.get(&format!("/crypto/holdings/{}/transactions", holding["asset_id"])).await.json();
    assert_eq!(transactions.as_array().unwrap().len(), 1);
    assert_eq!(money(&transactions[0]["price_per_unit"]), dec("45000"));
    assert_eq!(transactions[0]["note"], "typo fix");
}

#[tokio::test]
async fn test_update_transaction_rejects_editing_a_sell_to_exceed_holdings() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    let holding = add_bitcoin(&client, "2", "40000").await;
    client
        .post(
            &format!("/crypto/holdings/{}/transactions", holding["asset_id"]),
            json!({"type": "sell", "quantity": "1", "price_per_unit": "45000", "date": "2026-02-01"}),
        )
        .await;
    let transactions = client.get(&format!("/crypto/holdings/{}/transactions", holding["asset_id"])).await.json();
    let sell_tx_id = transactions.as_array().unwrap().iter().find(|t| t["type"] == "sell").unwrap()["id"].clone();

    let resp = client.patch(&format!("/crypto/transactions/{sell_tx_id}"), json!({"quantity": "3"})).await;

    assert_eq!(resp.status, 400);
}

#[tokio::test]
async fn test_refresh_updates_30d_and_1y_change() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    add_bitcoin(&client, "0.5", "40000").await;

    client.coingecko.serve_markets(&[(
        "bitcoin",
        MarketPoint {
            price: dec("60000"),
            change_1h: None,
            change_24h: None,
            change_7d: None,
            change_30d: Some(dec("15.5")),
            change_1y: Some(dec("120.25")),
        },
    )]);

    let resp = client.post_empty("/crypto/refresh").await;

    let holding = holdings(&resp.json())[0].clone();
    assert_eq!(money(&holding["price_change_30d"]), dec("15.5"));
    assert_eq!(money(&holding["price_change_1y"]), dec("120.25"));
}

/// The 90d Best/Worst Performer window can't ride along with the regular
/// sync (CoinGecko's /coins/markets has no 90d bucket) — it's a separate
/// on-demand fetch (/coins/{id}/market_chart), served here by the same fake
/// CoinGecko that serves /coins/markets elsewhere in this file.
#[tokio::test]
async fn test_90d_performance_reflects_a_real_market_chart_fetch() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    let holding = add_default_bitcoin(&client).await;

    // performance_90d checks require_api_key() itself, separately from
    // its per-coin 90d fetch — CI has no AURUM_COINGECKO_API_KEY configured
    // (unlike a local .env), so the route would 400 before ever reaching the
    // fetch; the test app is always built with a dummy key for this
    // (see common/mod.rs's base_settings).

    // 12.5% for bitcoin; any other coin 404s, i.e. no 90d change.
    client.coingecko.serve_90d_change(&[("bitcoin", 12.5)]);

    let resp = client.get("/crypto/performance/90d").await;

    assert_eq!(resp.status, 200, "{}", resp.text);
    assert_eq!(resp.json()["items"], json!([{"asset_id": holding["asset_id"], "price_change_percent": 12.5}]));
}

#[tokio::test]
async fn test_90d_performance_skips_fully_sold_holdings() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    let holding = add_bitcoin(&client, "1", "40000").await;
    let resp = client
        .post(
            &format!("/crypto/holdings/{}/transactions", holding["asset_id"]),
            json!({"type": "sell", "quantity": "1", "price_per_unit": "45000", "date": "2026-02-01"}),
        )
        .await;
    assert_eq!(resp.status, 201, "{}", resp.text);

    client.coingecko.record_chart_calls();
    client.coingecko.serve_90d_change(&[("bitcoin", 5.0)]);

    let resp = client.get("/crypto/performance/90d").await;

    assert_eq!(resp.json()["items"], json!([]));
    assert_eq!(client.coingecko.chart_calls(), Vec::<String>::new()); // a coin with zero quantity has nothing to fetch a price for
}

#[tokio::test]
async fn test_90d_performance_filters_by_portfolio() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    let portfolio_a = client.post("/crypto/portfolios", json!({"name": "A"})).await.json();
    let portfolio_b = client.post("/crypto/portfolios", json!({"name": "B"})).await.json();

    add_bitcoin_to_portfolio(&client, &portfolio_a["id"], "1").await;
    let holding_b = add_bitcoin_to_portfolio(&client, &portfolio_b["id"], "1").await;

    // API key: the test app always has a dummy one (see common/mod.rs).
    client.coingecko.serve_90d_change(&[("bitcoin", 3.0)]);

    let resp = client.get(&format!("/crypto/performance/90d?portfolio_id={}", portfolio_b["id"])).await;

    assert_eq!(resp.status, 200, "{}", resp.text);
    let items = resp.json()["items"].clone();
    let asset_ids: Vec<Value> = items.as_array().unwrap().iter().map(|item| item["asset_id"].clone()).collect();
    assert_eq!(asset_ids, vec![holding_b["asset_id"].clone()]);
}

#[tokio::test]
async fn test_holdings_are_sorted_by_invested_amount_descending() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[
        ("bitcoin", price("50000")),
        ("ethereum", price("2000")),
        ("litecoin", price("60")),
    ]);

    async fn add(client: &Api, coingecko_id: &str, symbol: &str, quantity: &str, price_per_unit: &str) {
        let resp = client
            .post(
                "/crypto/holdings",
                json!({
                    "coingecko_id": coingecko_id,
                    "symbol": symbol,
                    "name": symbol,
                    "quantity": quantity,
                    "price_per_unit": price_per_unit,
                    "date": "2026-01-01",
                }),
            )
            .await;
        assert_eq!(resp.status, 201, "{}", resp.text);
    }

    // Cost basis: LTC 100, BTC 5000, ETH 1000 — expect BTC, ETH, LTC.
    add(&client, "litecoin", "ltc", "2", "50").await;
    add(&client, "bitcoin", "btc", "0.1", "50000").await;
    add(&client, "ethereum", "eth", "1", "1000").await;

    let resp = client.get("/crypto/holdings").await;

    let symbols: Vec<Value> = holdings(&resp.json()).iter().map(|h| h["symbol"].clone()).collect();
    assert_eq!(symbols, vec!["BTC", "ETH", "LTC"]);
}

#[tokio::test]
async fn test_fully_sold_holding_sorts_last() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000")), ("litecoin", price("60"))]);
    let sold = add_bitcoin(&client, "1", "40000").await;
    let resp = client
        .post(
            &format!("/crypto/holdings/{}/transactions", sold["asset_id"]),
            json!({"type": "sell", "quantity": "1", "price_per_unit": "45000", "date": "2026-02-01"}),
        )
        .await;
    assert_eq!(resp.status, 201, "{}", resp.text);
    let resp = client
        .post(
            "/crypto/holdings",
            json!({"coingecko_id": "litecoin", "symbol": "ltc", "name": "Litecoin", "quantity": "1", "price_per_unit": "50", "date": "2026-01-01"}),
        )
        .await;
    assert_eq!(resp.status, 201, "{}", resp.text);

    let resp = client.get("/crypto/holdings").await;

    let symbols: Vec<Value> = holdings(&resp.json()).iter().map(|h| h["symbol"].clone()).collect();
    assert_eq!(symbols, vec!["LTC", "BTC"]); // BTC has no cost basis left (fully sold)
}

#[tokio::test]
async fn test_update_transaction_404_for_unknown_id() {
    let client = TestApp::start().await;
    let resp = client.patch("/crypto/transactions/999999", json!({"quantity": "1"})).await;

    assert_eq!(resp.status, 404);
}

#[tokio::test]
async fn test_holding_without_portfolio_id_lands_in_an_auto_created_default() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    let holding = add_default_bitcoin(&client).await;

    let portfolios = client.get("/crypto/portfolios").await.json();
    assert_eq!(portfolios.as_array().unwrap().len(), 1);
    assert_eq!(portfolios[0]["name"], "Main Portfolio");
    assert_eq!(holding["portfolio_id"], portfolios[0]["id"]);
}

#[tokio::test]
async fn test_create_portfolio_and_file_a_holding_under_it() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);

    let created = client.post("/crypto/portfolios", json!({"name": "Long-term"})).await;
    assert_eq!(created.status, 201, "{}", created.text);
    let portfolio = created.json();
    assert_eq!(portfolio["name"], "Long-term");
    assert_eq!(portfolio["is_archived"], json!(false));

    let holding = add_bitcoin_to_portfolio(&client, &portfolio["id"], "0.5").await;
    assert_eq!(holding["portfolio_id"], portfolio["id"]);
}

#[tokio::test]
async fn test_same_coin_can_be_tracked_separately_in_two_portfolios() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    let memes = client.post("/crypto/portfolios", json!({"name": "Memes"})).await.json();
    let longterm = client.post("/crypto/portfolios", json!({"name": "Long-term"})).await.json();

    add_bitcoin_to_portfolio(&client, &memes["id"], "1").await;
    add_bitcoin_to_portfolio(&client, &longterm["id"], "2").await;

    let all_holdings = client.get("/crypto/holdings").await.json();
    assert_eq!(holdings(&all_holdings).len(), 2);

    let memes_holdings = client.get(&format!("/crypto/holdings?portfolio_id={}", memes["id"])).await.json();
    assert_eq!(holdings(&memes_holdings).len(), 1);
    assert_eq!(money(&holdings(&memes_holdings)[0]["quantity"]), dec("1"));

    let longterm_history =
        client.get(&format!("/crypto/history?range=30d&portfolio_id={}", longterm["id"])).await.json();
    assert_eq!(money(&longterm_history["current"]), dec("100000")); // 2 * 50000, memes' 1 BTC excluded
}

#[tokio::test]
async fn test_deleting_a_nonempty_portfolio_is_rejected() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    let holding = add_default_bitcoin(&client).await;
    let portfolio_id = holding["portfolio_id"].clone();

    let resp = client.delete(&format!("/crypto/portfolios/{portfolio_id}")).await;

    assert_eq!(resp.status, 400);
}

#[tokio::test]
async fn test_archived_portfolios_are_excluded_by_default() {
    let client = TestApp::start().await;
    let portfolio = client.post("/crypto/portfolios", json!({"name": "Old fund"})).await.json();

    client.patch(&format!("/crypto/portfolios/{}", portfolio["id"]), json!({"is_archived": true})).await;

    let visible = client.get("/crypto/portfolios").await.json();
    assert_eq!(visible, json!([]));

    let with_archived = client.get("/crypto/portfolios?include_archived=true").await.json();
    assert_eq!(with_archived.as_array().unwrap().len(), 1);
    assert_eq!(with_archived[0]["is_archived"], json!(true));
}

#[tokio::test]
async fn test_empty_archived_portfolio_can_be_deleted() {
    let client = TestApp::start().await;
    let portfolio = client.post("/crypto/portfolios", json!({"name": "Unused"})).await.json();

    let resp = client.delete(&format!("/crypto/portfolios/{}", portfolio["id"])).await;

    assert_eq!(resp.status, 204);
    assert_eq!(client.get("/crypto/portfolios").await.json(), json!([]));
}

#[tokio::test]
async fn test_create_holding_defaults_risk_level_to_high() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);

    let holding = add_default_bitcoin(&client).await;

    assert_eq!(holding["risk_level"], "high");
}

#[tokio::test]
async fn test_create_holding_accepts_a_custom_risk_level() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);

    let holding = add_bitcoin_with(&client, "0.5", "40000", Some("low"), None).await;

    assert_eq!(holding["risk_level"], "low");
}

/// risk_level lives on the Asset, not CryptoHolding — see
/// services/crypto.rs's hydrate — so it's edited through the
/// existing PATCH /assets/{asset_id}, same endpoint every other Asset
/// already uses, rather than a crypto-specific route.
#[tokio::test]
async fn test_holding_risk_level_is_editable_via_the_underlying_asset() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    let holding = add_bitcoin_with(&client, "0.5", "40000", Some("high"), None).await;

    let resp = client.patch(&format!("/assets/{}", holding["asset_id"]), json!({"risk_level": "medium"})).await;
    assert_eq!(resp.status, 200, "{}", resp.text);

    let refreshed = holdings(&client.get("/crypto/holdings").await.json())[0].clone();
    assert_eq!(refreshed["risk_level"], "medium");
}

#[tokio::test]
async fn test_create_holding_defaults_network_to_null() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);

    let holding = add_default_bitcoin(&client).await;

    assert!(holding["network"].is_null());
}

#[tokio::test]
async fn test_create_holding_accepts_a_network() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);

    let holding = add_bitcoin_with(&client, "0.5", "40000", None, Some("Ethereum")).await;

    assert_eq!(holding["network"], "Ethereum");
}

#[tokio::test]
async fn test_update_holding_network() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    let holding = add_default_bitcoin(&client).await;

    let resp = client.patch(&format!("/crypto/holdings/{}", holding["asset_id"]), json!({"network": "Tron"})).await;

    assert_eq!(resp.status, 200, "{}", resp.text);
    assert_eq!(resp.json()["network"], "Tron");
    let refreshed = holdings(&client.get("/crypto/holdings").await.json())[0].clone();
    assert_eq!(refreshed["network"], "Tron");
}

#[tokio::test]
async fn test_update_holding_network_can_be_cleared_with_null() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    let holding = add_bitcoin_with(&client, "0.5", "40000", None, Some("Ethereum")).await;

    let resp = client.patch(&format!("/crypto/holdings/{}", holding["asset_id"]), json!({"network": null})).await;

    assert_eq!(resp.status, 200, "{}", resp.text);
    assert!(resp.json()["network"].is_null());
}

/// Partial PATCH semantics, same as every other endpoint in this app —
/// an empty body must not silently null out a field the caller didn't
/// mention (see aurum-domain's CryptoHoldingUpdate).
#[tokio::test]
async fn test_update_holding_omitting_network_leaves_it_untouched() {
    let client = TestApp::start().await;
    client.coingecko.serve_markets(&[("bitcoin", price("50000"))]);
    let holding = add_bitcoin_with(&client, "0.5", "40000", None, Some("Ethereum")).await;

    let resp = client.patch(&format!("/crypto/holdings/{}", holding["asset_id"]), json!({})).await;

    assert_eq!(resp.status, 200, "{}", resp.text);
    assert_eq!(resp.json()["network"], "Ethereum");
}

#[tokio::test]
async fn test_update_holding_404_for_unknown_id() {
    let client = TestApp::start().await;
    let resp = client.patch("/crypto/holdings/999999", json!({"network": "Ethereum"})).await;

    assert_eq!(resp.status, 404);
}
