//! The black-box API suite, ported from the former Python backend's `tests` (pytest): one module
//! per pytest module, one `#[tokio::test]` per pytest test with the same
//! name, the same requests and the same assertions. The harness
//! (`common/`) builds the app in-process against a fresh database per test;
//! see `README.md` for how to run it.
//!
//! All modules form one test binary (`autotests = false` in Cargo.toml), so
//! the server crate is linked once instead of once per file.

mod common;

mod test_accounts;
mod test_app_settings_seed;
mod test_backup;
mod test_budgets;
mod test_cash_flow;
mod test_categories;
mod test_cors;
mod test_crypto;
mod test_dashboard;
mod test_fx_conversion;
mod test_goals;
mod test_insights;
mod test_reports;
mod test_tags;
mod test_transactions;
