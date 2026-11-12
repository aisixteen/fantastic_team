//! The verbs the puller declares: one struct per verb, then `verbs!` — the list the
//! agent's own entry checks every call against and `reflect` and `docs` render.

use schemars::JsonSchema;
use serde::Deserialize;

/// Grant the team's views what the puller's agents need (sent by the kernel at start).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Boot {}

/// One pass over the accepted issues. Returns {created: [repo#n], skipped: [[repo#n, reason]]}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pull {}

/// The figures of the passes since the process started (last_pull_s is unix seconds, 0 before the first); no login and no token. Returns {repos, github, store, pulls, last_pull_s, last_created, last_skipped, total_created}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Status {}

fantastic_kernel::verbs! {
    "boot" => Boot,
    "pull" => Pull,
    "status" => Status,
}
