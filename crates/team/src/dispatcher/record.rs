//! The fields a dispatcher's record carries, beside the ones every record has.

use schemars::JsonSchema;

/// A dispatcher's own record fields.
#[derive(Debug, JsonSchema)]
pub struct Record {
    /// The board agent the dispatcher reads the work from.
    #[serde(default = "board")]
    #[schemars(length(min = 1))]
    pub board: String,
    /// The team's name (the root of its tree).
    #[serde(default = "team")]
    #[schemars(length(min = 1))]
    pub team: String,
    /// Minutes without movement before a task counts as stalled.
    #[serde(default = "stall_minutes")]
    #[schemars(range(min = 1))]
    pub stall_minutes: u64,
    /// The roles registry agent (roles mode).
    #[schemars(length(min = 1))]
    pub roles: Option<String>,
    /// The groups: group name → its workers' ids.
    pub groups: Option<std::collections::BTreeMap<String, Vec<String>>>,
}

fn board() -> String {
    "board".to_string()
}

fn team() -> String {
    "team".to_string()
}

fn stall_minutes() -> u64 {
    30
}

fantastic_kernel::record!(Record);
