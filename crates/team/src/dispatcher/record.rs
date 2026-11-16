//! The fields a dispatcher's record carries, beside the ones every record has.

use schemars::JsonSchema;

/// A dispatcher's own record fields.
#[derive(Debug, JsonSchema)]
pub struct Record {
    /// The team's name (the root of its tree).
    #[serde(default = "team")]
    #[schemars(length(min = 1))]
    pub team: String,
    /// The bundles registry agent (bundles mode).
    #[schemars(length(min = 1))]
    pub bundles: Option<String>,
    /// The groups: group name → its workers' ids.
    pub groups: Option<std::collections::BTreeMap<String, Vec<String>>>,
}

fn team() -> String {
    "team".to_string()
}

fantastic_kernel::record!(Record);
