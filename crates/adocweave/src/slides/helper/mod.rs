//! Optional one-shot Node helper; no manuscript-controlled executable discovery.

mod prepare;
mod process;
pub mod protocol;
mod validation;

use adocweave_core::text::TextRange;
use std::collections::BTreeMap;
use std::fmt;

pub use prepare::{Prepared, Selection, prepare};
pub use process::{ProcessLimits, execute, execute_sync, resolve_executable};
pub use protocol::{Csl, Macro, Scope};
pub use validation::{ValidatedResults, validate_response};

pub type HostResult<T> = Result<T, HostError>;
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostError {
    pub code: &'static str,
    pub message: String,
    pub range: Option<TextRange>,
}
impl HostError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            range: None,
        }
    }
    pub fn at(mut self, range: Option<TextRange>) -> Self {
        self.range = range;
        self
    }
    pub fn protocol(message: impl Into<String>) -> Self {
        Self::new("slides-helper-protocol", message)
    }
}
impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for HostError {}
#[derive(Clone, Debug)]
pub struct HostDiagnostic {
    pub scope: Option<Scope>,
    pub key: Option<String>,
    pub range: Option<TextRange>,
    pub severity: protocol::Severity,
    pub code: String,
    pub message: String,
}
pub type SourceMap = BTreeMap<(Scope, String), TextRange>;
