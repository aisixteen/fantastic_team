//! The fields a board's record carries, beside the ones every record has.

use schemars::JsonSchema;

/// A board's own record fields.
#[derive(Debug, JsonSchema)]
pub struct Record {
    /// The repositories the board follows, `owner/name`.
    pub repos: Option<Vec<String>>,
    /// The github connector agent the issues are read through.
    #[schemars(length(min = 1), regex(pattern = r"\S"))]
    pub github: String,
    /// The store agent the board's file is kept in.
    #[schemars(length(min = 1), regex(pattern = r"\S"))]
    pub store: String,
    /// Who an `ask` goes to when it names no one.
    #[serde(default = "ask_to")]
    #[schemars(length(min = 1), regex(pattern = r"\S"))]
    pub ask_to: String,
    /// The bundles registry agent (bundles mode): the board then also shows its tree.
    #[schemars(length(min = 1), regex(pattern = r"\S"))]
    pub bundles: Option<String>,
    /// Minutes without movement before the audit calls a task stale.
    #[serde(default = "stale_minutes")]
    #[schemars(range(min = 1))]
    pub stale_minutes: u64,
}

fn ask_to() -> String {
    "user".to_string()
}

fn stale_minutes() -> u64 {
    30
}

fantastic_kernel::record!(Record);
