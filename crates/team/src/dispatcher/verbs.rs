//! The verbs the dispatcher declares: one struct per verb, then `verbs!` — the list the
//! agent's own entry checks every call against and `reflect` and `docs` render.

use schemars::JsonSchema;
use serde::Deserialize;

/// Grant the team's views what the dispatcher's agents need (sent by the kernel at start).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Boot {}

/// Per group one flat list: each worker, its subagents expander, and its subagents when expanded; with a roles registry the reply also has fleet {running, max}, the registry's get_count, and (roles mode) agents, the lead first then its tree depth-first [{id, role, state, depth, indent, guides, lead, label}], and plan {lead, fleet, tasks}; in every mode services [{id, kind: queue|board|github|dispatcher|cadencer|puller|git, label}], the team's own agents that exist.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sidebar {
    /// The workers whose subagents are shown.
    pub expanded: Option<Vec<String>>,
}

/// The tree of the team or a node, rows with state/task/minutes/cost.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Tree {
    /// The node; absent, the team.
    #[schemars(length(min = 1))]
    pub node: Option<String>,
}

/// The view to open for a node: a worker or subagent → full-screen chat; a group or the team → a dispatcher; a node in the sidebar's services → {view: panel, kind, target, model: {kind, title, summary, sections}} drawn as text in the right pane.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Open {
    /// The node.
    #[schemars(length(min = 1))]
    pub node: String,
}

/// What `control` does to a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ControlAction {
    /// Start the node.
    Start,
    /// Pause the node.
    Pause,
    /// Stop the node (needs `confirm`).
    Stop,
    /// Send a worker `task <task>`.
    HandTask,
    /// Make a worker in a group.
    AddWorker,
    /// Delete a worker (needs `confirm`).
    RemoveWorker,
}

impl ControlAction {
    /// The action as a call spells it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ControlAction::Start => "start",
            ControlAction::Pause => "pause",
            ControlAction::Stop => "stop",
            ControlAction::HandTask => "hand_task",
            ControlAction::AddWorker => "add_worker",
            ControlAction::RemoveWorker => "remove_worker",
        }
    }
}

/// Act on a node. Returns {sent, error?} (or a question for stop and remove_worker without confirm); the node's refusal or no reply is sent: false with its error. hand_task {node: worker, task} sends the worker `task <task>`. add_worker {node: group, id, template} makes the worker from the tier named `template` in the roles registry's record (`tiers`), its memory in the store that tier names, and lists it in the group (the dispatcher may send, stop, start and pause it at once; its chat grant follows at the next start). remove_worker {node: worker, confirm} deletes the worker (its rows stay in the store) and drops it from its group.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Control {
    /// The worker, group or team acted on.
    #[schemars(length(min = 1))]
    pub node: String,
    /// What to do.
    pub action: ControlAction,
    /// `true` confirms a stop or a remove_worker.
    #[serde(default)]
    pub confirm: bool,
    /// With hand_task: the task.
    pub task: Option<String>,
    /// With add_worker: the new worker's id.
    pub id: Option<String>,
    /// With add_worker: the tier it is made from.
    pub template: Option<String>,
}

fantastic_kernel::verbs! {
    "boot" => Boot,
    "sidebar" => Sidebar,
    "tree" => Tree,
    "open" => Open,
    "control" => Control,
}
