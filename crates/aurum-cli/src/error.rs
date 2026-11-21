//! The single error type of the CLI: a ready-to-print message. `main` writes
//! it to stderr and exits with code 1, like the Node CLI did.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError(pub String);

impl CliError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CliError {}

pub type Result<T> = std::result::Result<T, CliError>;
