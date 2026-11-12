//! The verbs the roles registry declares: one struct per verb, then `verbs!` — the list
//! the agent's own entry checks every call against and `reflect` and `docs` render.

use schemars::JsonSchema;
use serde::Deserialize;

/// Every role: each meta bundle (bundles/<name>/bundle.yaml) whose record carries a directive; `reflect` with bundles:'all' lists the same bundles. Read on each call. Returns {roles:[{name, purpose}], errors:[...]}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Roles {}

/// One role. Returns {name, purpose, grants, directive}; an unknown name is `not_found`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Role {
    /// The role's name (a role bundle).
    #[schemars(length(min = 1))]
    pub name: String,
}

/// Part of YOUR quota handed to the child as its own subtree cap.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    /// How many agents the child's subtree may hold in all.
    pub total: Option<u64>,
    /// How many of one role the child's subtree may hold.
    pub per_role: Option<u64>,
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

/// Creates the agent from the role bundle (its directive after the default system block, its grants), an AI chat completing through `provider` on `model`, as a CHILD OF THE CALLER, gives the caller pause/resume/stop on it, and lets the child `send` the caller (both ends, merged with what either already holds), so it tells its parent with a message when it is done. The child's task comes FROM this registry, so a child cannot tell its parent from it: AFTER ALL YOUR SPAWNS, send each child the team's structure (`send` with `text`): you are its parent (your id, the one it reports to with `send` when done), and every sibling's id, role and task. Refused past a limit: {error:<sentence>, reason:'limit', scope, role, running, max, hint}. `from` branches the new agent off a fork point of `agent` instead (ai_core `fork {at}`, same memory store, so it starts with that conversation up to `at`), then applies the card and limits; it sits BESIDE its source, not under you, and you still get pause/resume/stop on it and it may `send` you; you must hold `fork` on `agent` or it is refused with reason `not_allowed`; a refusal from fork (cuts_turn, cuts_fold, no_such_row, parent_mid_turn, ...) passes through unchanged and nothing is left behind. Then the reply also has from:{agent, at} and copied. The card's grants on tools are opened on both ends (the child's own grant and the tool's callable_by, all or nothing): a card may only grant what the spawner itself holds, or the spawn is refused with reason `not_allowed`, and a grant that cannot be written undoes the whole spawn (reason `grant_failed`). Returns {id, role, provider, model}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Spawn {
    /// The role to spawn (a role bundle).
    #[schemars(length(min = 1))]
    pub role: String,
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
    /// Part of your quota handed to the child.
    pub limits: Option<Limits>,
    /// Branch off a fork point of an existing agent.
    pub from: Option<ForkFrom>,
}

/// The agents this registry spawned, per role, read from the records (survives restart). Returns {roles:{<role>:[{id}]}}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fleet {}

/// Cap one role, or all agents in total; no `max` clears the cap. Returns {ok:true, role, max}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetLimit {
    /// The role to cap; omitted = all agents in total.
    #[schemars(length(min = 1))]
    pub role: Option<String>,
    /// The cap; omitted (or null), the cap is cleared.
    pub max: Option<u64>,
}

/// How many agents this registry spawned still exist (a slot frees on delete_agent). Returns {running, max}; max is null when uncapped.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCount {
    /// The role to count; omitted = in total.
    #[schemars(length(min = 1))]
    pub role: Option<String>,
}

fantastic_kernel::verbs! {
    "roles" => Roles,
    "role" => Role,
    "spawn" => Spawn,
    "fleet" => Fleet,
    "set_limit" => SetLimit,
    "get_count" => GetCount,
}
