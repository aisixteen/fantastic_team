//! The verbs the cadencer declares: one struct per verb, then `verbs!` — the list the
//! agent's own entry checks every call against and `reflect` and `docs` render.

use schemars::JsonSchema;
use serde::Deserialize;

/// Load the settings from the agent record and restart the timer when it is enabled with bundles; grant the team's views what the cadencer's agents need. Returns {loaded}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Boot {}

/// One agent of the tree a tick is given.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TreeNode {
    /// The agent's id.
    #[schemars(length(min = 1))]
    pub id: String,
    /// Its parent's id.
    pub parent: Option<String>,
    /// Whether it is an AI.
    #[serde(default = "yes")]
    pub has_ai: bool,
    /// Who drives it.
    #[serde(default)]
    pub driver: String,
    /// Its state.
    #[serde(default)]
    pub state: String,
    /// Its task.
    #[serde(default)]
    pub task: String,
    /// Seconds idle.
    #[serde(default)]
    pub idle_s: u64,
    /// Seconds on the task.
    #[serde(default)]
    pub time_on_task_s: u64,
    /// What it cost.
    #[serde(default)]
    pub cost: f64,
    /// Its cache hit rate.
    #[serde(default)]
    pub cache_hit: f64,
    /// Results it has not shown yet.
    #[serde(default)]
    pub new_results: u64,
}

fn yes() -> bool {
    true
}

/// Beat once with this tree. Returns {digests: [{to, lines, text}], sent}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Tick {
    /// The tree of the team.
    pub tree: Vec<TreeNode>,
}

/// Change the settings; the sender must be the parent, maintainer, user, the CLI or the team page. They are persisted in the agent record and restored at boot. Returns the settings.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Set {
    /// Whether the timer runs.
    pub enabled: Option<bool>,
    /// Seconds between beats.
    #[schemars(range(min = 1))]
    pub cadence: Option<u64>,
    /// Seconds idle before a worker with a task counts as stalled.
    pub stall_after: Option<u64>,
    /// Whether a quiet lead gets only changes and a heartbeat.
    pub quiet: Option<bool>,
    /// A lead whose digests `on` switches.
    #[schemars(length(min = 1))]
    pub lead: Option<String>,
    /// With `lead`: whether its digests are sent.
    #[serde(default = "yes")]
    pub on: bool,
}

/// Name the parent (kept as `cadencer_parent`, persisted in the agent record and restored at boot); the timer restarts. Returns {parent}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Configure {
    /// The parent's id.
    #[schemars(length(min = 1))]
    pub parent: Option<String>,
}

/// The cadencer's state. Returns {settings, ticks, last_beat_s, next_beat_s, digests_sent, digest_bytes_per_hour, load}; load is the bundles registry's counts (workers busy/idle/stalled per group, tasks held/in_review/done/blocked/queued, done_last_hour) and carries no mode.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Status {}

fantastic_kernel::verbs! {
    "boot" => Boot,
    "tick" => Tick,
    "set" => Set,
    "configure" => Configure,
    "status" => Status,
}
