//! SQL, one module per route group. Plain runtime-checked `sqlx::query*`
//! calls (no compile-time macros, so building never needs a database);
//! functions return `sqlx::Result` and know nothing about HTTP.

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
