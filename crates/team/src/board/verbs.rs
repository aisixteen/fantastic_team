//! The verbs the board declares: one struct per verb, then `verbs!` — the list the
//! agent's own entry checks every call against and `reflect` and `docs` render.

use crate::board::Status;
use schemars::JsonSchema;
use serde::Deserialize;

/// Grant the team's views what the board's agents need (sent by the kernel at start).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Boot {}

/// Pull the accepted issues into the board. Returns {tasks: n, errors: [..]}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pull {}

/// The board. Returns {tasks: {pending, in_work, done, failed}, agents: {..}, roles?}; roles (roles mode) = {lead, fleet, tree, agents: [{id, role, state, parent}], tasks: [{id, source, holder, status}]}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Board {}

/// One task. Returns the task.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Task {
    /// The task's id, `repo#number`.
    #[schemars(length(min = 1))]
    pub id: String,
}

/// Report a task's progress. Returns {ok}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Report {
    /// The task's id, `repo#number`.
    #[schemars(length(min = 1))]
    pub task: String,
    /// Where the task stands.
    pub status: Status,
    /// A note.
    #[serde(default)]
    pub note: String,
    /// The pull request.
    pub pr: Option<String>,
    /// Who reports.
    #[serde(default = "unknown")]
    pub author: String,
}

fn unknown() -> String {
    "unknown".to_string()
}

/// An agent's state and cost. Returns {ok}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    /// The agent's name.
    #[schemars(length(min = 1))]
    pub name: String,
    /// What it is doing.
    #[serde(default)]
    pub state: String,
    /// What it cost so far.
    #[serde(default)]
    pub cost: f64,
}

/// Ask a question and record the answer. Returns the answer.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Ask {
    /// Who to ask; absent, the record's `ask_to`.
    #[schemars(length(min = 1))]
    pub to: Option<String>,
    /// The question.
    #[schemars(length(min = 1))]
    pub question: String,
}

/// Audit the board and ask each finding of the record's `ask_to`. Returns {findings: [..]}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Audit {}

fantastic_kernel::verbs! {
    "boot" => Boot,
    "pull" => Pull,
    "board" => Board,
    "task" => Task,
    "report" => Report,
    "agent" => Agent,
    "ask" => Ask,
    "audit" => Audit,
}
