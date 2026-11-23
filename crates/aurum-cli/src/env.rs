//! Snapshot of the process environment. Passed around explicitly (the Node
//! CLI took an `environment` argument for the same reason) so tests can
//! supply their own variables without touching the real process state.

use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct Env(HashMap<String, String>);

impl Env {
    pub fn from_process() -> Self {
        Self(
            std::env::vars_os()
                .map(|(k, v)| (k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned()))
                .collect(),
        )
    }

    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        Self(pairs.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
    }

    /// The variable's value, `None` when unset (an empty value is `Some("")`).
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }
}
