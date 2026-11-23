//! Audit trail for the handful of operations that destroy or wholesale
//! replace data — port of the former Python backend's `app/core/audit.py`.
//!
//! Aurum is single-user, so this isn't about "who did it" — there's only one
//! person. It's about *what happened and when*, after the fact: restoring a
//! backup wipes every table before writing the new rows, deleting an account
//! takes its transactions with it, and a CSV import can add thousands of rows
//! in one request. When someone opens the app and finds it empty, the
//! container log is the only place that can say whether a restore ran at
//! 02:14 or whether something else went wrong.
//!
//! Deliberately just the standard log, no table of its own: an audit trail
//! that lives in the same database it's auditing disappears in exactly the
//! event it exists to explain (a restore truncates it too). Read it with
//! `docker compose logs backend`.

/// Record one irreversible operation. `details` are rendered as key=value;
/// keep them to counts and identifiers — never transaction descriptions,
/// amounts or anything else that would spill the user's finances into a log
/// file that gets shared when debugging.
pub fn log_destructive(action: &str, details: &[(&str, &dyn std::fmt::Display)]) {
    if details.is_empty() {
        tracing::warn!(target: "aurum.audit", "{action}");
    } else {
        let rendered: Vec<String> = details.iter().map(|(k, v)| format!("{k}={v}")).collect();
        tracing::warn!(target: "aurum.audit", "{action} {}", rendered.join(" "));
    }
}
