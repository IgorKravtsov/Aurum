//! Business logic, one module per route group. Services take `&PgPool`
//! (or a transaction), call `repo::*`, and return `ApiResult<T>` so they can
//! raise FastAPI-style HTTP errors exactly where the Python code did.

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
pub mod insights;
pub mod net_worth;
pub mod recurring;
pub mod reports;
pub mod settings;
pub mod tags;
pub mod transactions;
