//! The fields a roles registry's record carries, beside the ones every record has.

use schemars::JsonSchema;

/// The caps the registry holds, kept by `set_limit`.
#[derive(Debug, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    /// How many agents all roles together may hold.
    pub total: Option<u64>,
    /// How many agents each named role may hold, by role.
    pub per_role: Option<std::collections::BTreeMap<String, u64>>,
}

/// A roles registry's own record fields.
#[derive(Debug, JsonSchema)]
pub struct Record {
    /// The registry's caps; kept by `set_limit`.
    pub limits: Option<Limits>,
    /// The tiers a team may make a worker from, by name: each the fields of the worker's record.
    pub tiers: Option<std::collections::BTreeMap<String, serde_json::Map<String, serde_json::Value>>>,
}

fantastic_kernel::record!(Record);
