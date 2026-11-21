//! Route groups, one module per `backend/app/api/routes/*.py` file. Each
//! exposes `pub fn router() -> OpenApiRouter<AppState>` and owns its paths;
//! this file only merges them, so porting a group never touches it.

pub mod accounts;
pub mod advice;
pub mod assets;
pub mod backup;
pub mod budgets;
pub mod cash_flow;
pub mod categories;
pub mod crypto;
pub mod dashboard;
pub mod goals;
pub mod health;
pub mod insights;
pub mod net_worth;
pub mod recurring;
pub mod reports;
pub mod settings;
pub mod tags;
pub mod transactions;

use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

/// Every API route plus its OpenAPI description, in main.py's include order.
pub fn api_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(dashboard::router())
        .merge(accounts::router())
        .merge(categories::router())
        .merge(transactions::router())
        .merge(assets::router())
        .merge(net_worth::router())
        .merge(backup::router())
        .merge(reports::router())
        .merge(insights::router())
        .merge(settings::router())
        .merge(budgets::router())
        .merge(advice::router())
        .merge(goals::router())
        .merge(recurring::router())
        .merge(cash_flow::router())
        .merge(tags::router())
        .merge(crypto::router())
        .merge(health::router())
}
