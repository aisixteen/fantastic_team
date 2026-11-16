//! The verbs the dispatcher declares: one struct per verb, then `verbs!` — the list the
//! agent's own entry checks every call against and `reflect` and `docs` render.

use schemars::JsonSchema;
use serde::Deserialize;

/// Grant the team's views what the dispatcher's agents need (sent by the kernel at start).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Boot {}

/// Grant the page's leg what opening a node takes, and name it: a worker or subagent → its full-screen chat (the chat verbs); an agent whose reflect carries an html view → {view: html} (reflect and the view's calls); a file's view agent → {kind: file} (reflect, read, watch); a group or the team → a dispatcher.
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

/// Act on a node. Returns {sent, error?} (or a question for stop and remove_worker without confirm); the node's refusal or no reply is sent: false with its error. hand_task {node: worker, task} sends the worker `task <task>`. add_worker {node: group, id, template} makes the worker from the tier named `template` in the bundles registry's record (`tiers`), its memory in the store that tier names, and lists it in the group (the dispatcher may send, stop, start and pause it at once; its chat grant follows at the next start). remove_worker {node: worker, confirm} deletes the worker (its rows stay in the store) and drops it from its group.
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
    "open" => Open,
    "control" => Control,
}
