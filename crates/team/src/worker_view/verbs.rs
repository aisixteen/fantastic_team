//! The verbs the worker view declares: one struct per verb, then `verbs!` — the list the
//! agent's own entry checks every call against and `reflect` and `docs` render.

use schemars::JsonSchema;
use serde::Deserialize;

/// Open a worker's view: creates it, or returns the existing one. Returns the view.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Open {
    /// The worker agent's id.
    #[schemars(length(min = 1))]
    pub worker: String,
    /// The group the view cycles within for `next` and `prev`; absent, the default group.
    #[schemars(length(min = 1))]
    pub group: Option<String>,
}

/// The default cell set of a worker. Returns {cells}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Cells {
    /// The worker agent's id.
    #[schemars(length(min = 1))]
    pub worker: String,
}

/// A cell to add to a view.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlacedCell {
    /// What the cell shows: `task`, `files`, `output`, `preview` (never `chat`).
    pub kind: String,
    /// The heading shown on the cell.
    pub label: String,
    /// The agent the cell talks to.
    pub target: Option<String>,
    /// Ignored: a placed cell is never locked.
    #[serde(default)]
    pub locked: bool,
}

/// Add a cell to the top part of a view (before the locked chat cell). Returns the view.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Place {
    /// The worker agent's id (its view must be open).
    #[schemars(length(min = 1))]
    pub worker: String,
    /// The cell.
    pub cell: PlacedCell,
}

/// The next worker in the group of a worker's view, wrapping around. Returns {worker}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Next {
    /// The worker agent's id.
    #[schemars(length(min = 1))]
    pub worker: String,
}

/// The previous worker in the group of a worker's view, wrapping around. Returns {worker}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Prev {
    /// The worker agent's id.
    #[schemars(length(min = 1))]
    pub worker: String,
}

fantastic_kernel::verbs! {
    "open" => Open,
    "cells" => Cells,
    "place" => Place,
    "next" => Next,
    "prev" => Prev,
}
