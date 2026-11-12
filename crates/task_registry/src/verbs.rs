//! The verbs the task registry declares: one struct per verb, then `verbs!` — the list
//! the agent's own entry checks every call against and `reflect` and `docs` render.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The states a task takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// Not started.
    Waiting,
    /// Under way.
    Running,
    /// Held up.
    Blocked,
    /// Finished.
    Done,
    /// Failed.
    Failed,
    /// Paused.
    Paused,
}

impl State {
    /// The state as it is stored and spelled in a call.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            State::Waiting => "waiting",
            State::Running => "running",
            State::Blocked => "blocked",
            State::Done => "done",
            State::Failed => "failed",
            State::Paused => "paused",
        }
    }
}

/// Who holds a task: an existing agent, or a subtree named by its root agent.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Holder {
    /// An agent's id.
    Agent(#[schemars(length(min = 1))] String),
    /// A subtree.
    Subtree {
        /// The subtree's root agent.
        #[schemars(length(min = 1))]
        subtree: String,
    },
}

/// One repo a task touches (opaque strings).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Version {
    /// The repo.
    #[schemars(length(min = 1))]
    pub repo: String,
    /// The branch.
    #[schemars(length(min = 1))]
    pub branch: String,
    /// The commit.
    #[schemars(length(min = 1))]
    pub commit: String,
}

/// Where a task was forked from (opaque strings).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ForkFrom {
    /// The agent.
    #[schemars(length(min = 1))]
    pub agent: String,
    /// The mark.
    #[schemars(length(min = 1))]
    pub mark: String,
}

/// Token counts; a count left out is 0.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    /// Input tokens.
    #[serde(default)]
    pub input_tokens: u64,
    /// Cache-read tokens.
    #[serde(default)]
    pub cache_read_tokens: u64,
    /// Cache-write tokens.
    #[serde(default)]
    pub cache_write_tokens: u64,
    /// Output tokens.
    #[serde(default)]
    pub output_tokens: u64,
}

fn waiting() -> State {
    State::Waiting
}

/// Add a task. Refused `exists` (id taken), `not_found` (parent or holder). Returns the entry.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Add {
    /// The task's id.
    #[schemars(length(min = 1))]
    pub id: String,
    /// Where it came from, opaque: gh:o/r#12, jira:KEY-5, ...
    #[schemars(length(min = 1))]
    pub source: String,
    /// A task id it belongs under.
    #[schemars(length(min = 1))]
    pub parent: Option<String>,
    /// Where updates go, opaque; absent, the source.
    #[schemars(length(min = 1))]
    pub update_to: Option<String>,
    /// An existing agent, or a subtree.
    pub holder: Option<Holder>,
    /// Its starting state.
    #[serde(default = "waiting")]
    pub status: State,
    /// One per repo the task touches.
    pub versions: Option<Vec<Version>>,
    /// Opaque.
    #[schemars(length(min = 1))]
    pub source_version: Option<String>,
    /// Opaque.
    pub fork_from: Option<ForkFrom>,
}

/// Give a task nobody holds to a holder; else `already_held`. Each fact given replaces the entry's value (`versions: []` clears it). Returns the entry.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Assign {
    /// The task.
    #[schemars(length(min = 1))]
    pub id: String,
    /// An existing agent id, or a subtree named by its root agent.
    pub holder: Holder,
    /// One per repo the task touches.
    pub versions: Option<Vec<Version>>,
    /// Opaque.
    #[schemars(length(min = 1))]
    pub source_version: Option<String>,
    /// Opaque.
    pub fork_from: Option<ForkFrom>,
}

/// Move a held task to another holder (agent to agent, to a subtree and back); else `not_held`. Every step is kept in history {from, to, by, at}, plus the entry's non-empty versions, source_version and fork_from as set by that step. Returns the entry.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Reassign {
    /// The task.
    #[schemars(length(min = 1))]
    pub id: String,
    /// An existing agent id, or a subtree named by its root agent.
    pub holder: Holder,
    /// One per repo the task touches.
    pub versions: Option<Vec<Version>>,
    /// Opaque.
    #[schemars(length(min = 1))]
    pub source_version: Option<String>,
    /// Opaque.
    pub fork_from: Option<ForkFrom>,
}

/// The lead's way to update a task's state and facts. At least one of `status` and the three facts is required, else `missing_arg` for status. Returns the entry.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Status {
    /// The task.
    #[schemars(length(min = 1))]
    pub id: String,
    /// The new state.
    pub status: Option<State>,
    /// One per repo the task touches.
    pub versions: Option<Vec<Version>>,
    /// Opaque.
    #[schemars(length(min = 1))]
    pub source_version: Option<String>,
    /// Opaque.
    pub fork_from: Option<ForkFrom>,
}

/// Record a task's last report {from, state, result, at, usage}; a usage also becomes the entry's `usage` (the last one per task; a report without usage keeps it). `from` must be you or one of your descendants, else `not_in_subtree`. Returns the entry.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Report {
    /// The task.
    #[schemars(length(min = 1))]
    pub id: String,
    /// The reporting agent; absent, you.
    #[schemars(length(min = 1))]
    pub from: Option<String>,
    /// The state the reporter says the task is in.
    pub state: Option<State>,
    /// Anything.
    pub result: Option<Value>,
    /// Token counts.
    pub usage: Option<Usage>,
}

/// The tasks, sorted by id. Returns {tasks:[entry], usage:{input_tokens, cache_read_tokens, cache_write_tokens, output_tokens}}; usage is the sum of the returned tasks' last usage (none counts 0).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct List {
    /// Only the tasks under this one.
    #[schemars(length(min = 1))]
    pub parent: Option<String>,
    /// Only this holder's.
    pub holder: Option<Holder>,
    /// Only tasks in this state.
    pub status: Option<State>,
    /// Only these tasks; an id that is not a task is `not_found`.
    #[schemars(inner(length(min = 1)))]
    pub ids: Option<Vec<String>>,
}

/// One task plus `children` (the ids of the tasks whose parent it is); an unknown id is `not_found`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Get {
    /// The task.
    #[schemars(length(min = 1))]
    pub id: String,
}

fantastic_kernel::verbs! {
    "add" => Add,
    "assign" => Assign,
    "reassign" => Reassign,
    "status" => Status,
    "report" => Report,
    "list" => List,
    "get" => Get,
}
