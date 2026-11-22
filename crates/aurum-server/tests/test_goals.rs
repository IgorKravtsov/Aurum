//! Savings goals: contribution rules.
//!
//! The contribution log accepts negative amounts on purpose (taking money back
//! out of a goal is still an entry in its running total), which is exactly why
//! zero has to be rejected explicitly rather than by a `gt=0` constraint.

use serde_json::{Value, json};

use crate::common::{Api, TestApp, dec, money};

async fn goal(client: &Api) -> Value {
    client.post("/goals", json!({"name": "Emergency fund", "target_amount": "1000"})).await.json()
}

#[tokio::test]
async fn test_zero_contribution_is_rejected() {
    let client = TestApp::start().await;
    let goal = goal(&client).await;

    let resp = client
        .post(&format!("/goals/{}/contributions", goal["id"]), json!({"amount": "0", "date": "2026-01-15"}))
        .await;

    assert_eq!(resp.status, 422);
    assert_eq!(money(&client.get("/goals").await.json()[0]["current_amount"]), dec("0"));
}

/// Withdrawing from a goal is a legitimate entry — the zero check must not
/// turn into a positive-only constraint.
#[tokio::test]
async fn test_negative_contribution_is_allowed() {
    let client = TestApp::start().await;
    let goal = goal(&client).await;
    client.post(&format!("/goals/{}/contributions", goal["id"]), json!({"amount": "250", "date": "2026-01-15"})).await;

    let resp = client
        .post(&format!("/goals/{}/contributions", goal["id"]), json!({"amount": "-100", "date": "2026-02-01"}))
        .await;

    assert_eq!(resp.status, 201);
    assert_eq!(money(&resp.json()["current_amount"]), dec("150"));
}
