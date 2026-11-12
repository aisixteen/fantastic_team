//! The verbs the bundles registry declares: one struct per verb, then `verbs!` — the list
//! the agent's own entry checks every call against and `reflect` and `docs` render.

use schemars::JsonSchema;
use serde::Deserialize;

/// Every spawnable bundle: each meta bundle (bundles/<name>/bundle.yaml) whose record carries a directive; `reflect` with bundles:'all' lists the same bundles. Read on each call. Returns {bundles:[{name, purpose}], errors:[...]}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bundles {}

/// One spawnable bundle. Returns {name, purpose, grants, directive}; an unknown name is `not_found`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    /// The bundle's name.
    #[schemars(length(min = 1))]
    pub name: String,
}

/// A fork point of an existing agent.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ForkFrom {
    /// The agent to branch off.
    #[schemars(length(min = 1))]
    pub agent: String,
    /// The fork point (a row of its conversation).
    pub at: i64,
}

/// Creates the agent from the bundle (its directive after the default system block, its grants), an AI chat completing through `provider` on `model`, as a CHILD OF THE CALLER, gives the caller pause/resume/stop on it, and lets the child `send` the caller (both ends, merged with what either already holds), so it tells its parent with a message when it is done. The child's task comes FROM this registry, so a child cannot tell its parent from it: AFTER ALL YOUR SPAWNS, send each child the team's structure (`send` with `text`): you are its parent (your id, the one it reports to with `send` when done), and every sibling's id, bundle and task. `from` branches the new agent off a fork point of `agent` instead (ai_core `fork {at}`, same memory store, so it starts with that conversation up to `at`), then applies the card; it sits BESIDE its source, not under you, and you still get pause/resume/stop on it and it may `send` you; you must hold `fork` on `agent` or it is refused with reason `not_allowed`; a refusal from fork (cuts_turn, cuts_fold, no_such_row, parent_mid_turn, ...) passes through unchanged and nothing is left behind. Then the reply also has from:{agent, at} and copied. The card's grants on tools are opened on both ends (the child's own grant and the tool's callable_by, all or nothing): a card may only grant what the spawner itself holds, or the spawn is refused with reason `not_allowed`, and a grant that cannot be written undoes the whole spawn (reason `grant_failed`). Returns {id, bundle, provider, model}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Spawn {
    /// The bundle to spawn.
    #[schemars(length(min = 1))]
    pub bundle: String,
    /// The provider agent the child completes through; yours when omitted.
    #[schemars(length(min = 1))]
    pub provider: Option<String>,
    /// The model it runs there; yours when omitted, else the provider's default.
    #[schemars(length(min = 1))]
    pub model: Option<String>,
    /// Sent to the child as its first message.
    pub task: Option<String>,
    /// The child's display name.
    #[schemars(length(min = 1))]
    pub name: Option<String>,
    /// Branch off a fork point of an existing agent.
    pub from: Option<ForkFrom>,
}

/// The agents running on a spawnable bundle, per bundle, read from the tree (survives restart). Returns {bundles:{<bundle>:[{id}]}}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fleet {}

/// How many agents run on spawnable bundles, read from the tree. Returns {running}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCount {
    /// The bundle to count; omitted = all spawnable bundles together.
    #[schemars(length(min = 1))]
    pub bundle: Option<String>,
}

fantastic_kernel::verbs! {
    "bundles" => Bundles,
    "bundle" => Bundle,
    "spawn" => Spawn,
    "fleet" => Fleet,
    "get_count" => GetCount,
}
