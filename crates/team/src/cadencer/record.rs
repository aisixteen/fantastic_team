//! The fields the cadencer's record carries, beside the ones every record has: its
//! settings, kept by `set` and `configure` and restored at boot.

use schemars::JsonSchema;

/// What one group overrides of the global settings; an unset key follows them.
#[derive(Debug, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupSettings {
    /// Seconds between this group's beats.
    #[schemars(range(min = 1))]
    pub cadence: Option<u64>,
    /// Seconds idle before this group's workers count as stalled.
    pub stall_after: Option<u64>,
    /// Whether this group is quiet.
    pub quiet: Option<bool>,
    /// `false` switches the cadencer off for this group.
    pub enabled: Option<bool>,
}

/// The cadencer's own record fields.
#[derive(Debug, JsonSchema)]
pub struct Record {
    /// Whether the timer runs.
    #[serde(default = "enabled")]
    pub enabled: bool,
    /// Seconds between beats.
    #[serde(default = "cadence")]
    #[schemars(range(min = 1))]
    pub cadence: u64,
    /// Seconds idle before a worker with a task counts as stalled.
    #[serde(default = "stall_after")]
    pub stall_after: u64,
    /// Whether a quiet lead gets only changes and a heartbeat.
    #[serde(default = "quiet")]
    pub quiet: bool,
    /// The leads whose digests are switched off.
    pub off: Option<Vec<String>>,
    /// The groups' own settings, by group name (the lead's id).
    pub group_settings: Option<std::collections::BTreeMap<String, GroupSettings>>,
    /// The agent that drives the cadencer (`parent` is a reserved record key).
    #[schemars(length(min = 1))]
    pub cadencer_parent: Option<String>,
    /// The bundles registry agent (bundles mode): the digest tree is read from it.
    #[schemars(length(min = 1))]
    pub bundles: Option<String>,
}

fn enabled() -> bool {
    true
}

fn cadence() -> u64 {
    60
}

fn stall_after() -> u64 {
    300
}

fn quiet() -> bool {
    true
}

fantastic_kernel::record!(Record);
