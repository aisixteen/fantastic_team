//! The fields a bundles registry's record carries, beside the ones every record has.

use schemars::JsonSchema;

/// A bundles registry's own record fields.
#[derive(Debug, JsonSchema)]
pub struct Record {
    /// The tiers a team may make a worker from, by name: each the fields of the worker's record.
    pub tiers: Option<std::collections::BTreeMap<String, serde_json::Map<String, serde_json::Value>>>,
}

fantastic_kernel::record!(Record);
