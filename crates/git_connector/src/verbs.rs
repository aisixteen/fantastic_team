//! The verbs the git connector declares: one struct per verb, then `verbs!` — the list
//! the agent's own entry checks every call against and `reflect` and `docs` render.

use schemars::JsonSchema;
use serde::Deserialize;

/// Make a worktree on a new branch from a base. `path` is inside workspace_root (reflect names it; a relative path is joined to it). To use another folder, change workspace_root with update_agent on this record, and only after the user agrees to the new folder: ask first. Returns {path, branch, base, created}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorktreeAdd {
    /// The worktree's folder, inside workspace_root.
    #[schemars(length(min = 1))]
    pub path: String,
    /// The new branch.
    #[schemars(length(min = 1))]
    pub branch: String,
    /// The branch or commit it starts from.
    #[schemars(length(min = 1))]
    pub base: String,
}

/// Remove a worktree, never forcing. Returns {path, removed}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorktreeRemove {
    /// The worktree's folder, inside workspace_root.
    #[schemars(length(min = 1))]
    pub path: String,
}

/// Stage everything and commit with sign-off, the author and any trailers. Returns {path, commit, committed}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Commit {
    /// The worktree's folder, inside workspace_root.
    #[schemars(length(min = 1))]
    pub path: String,
    /// The commit message.
    #[schemars(length(min = 1))]
    pub message: String,
    /// Trailer lines, each `Key: value`.
    pub trailers: Option<Vec<String>>,
}

/// Push a work branch to the record's remote; never the default branch, `main` or `master`, and never a force. Returns {path, branch, remote, pushed}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Push {
    /// The worktree's folder, inside workspace_root.
    #[schemars(length(min = 1))]
    pub path: String,
    /// The work branch to push, a short name.
    #[schemars(length(min = 1))]
    pub branch: String,
    /// Never honoured: a force push is refused.
    pub force: Option<bool>,
}

/// Fetch the record's remote. Returns {remote, fetched}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fetch {}

/// The diffstat of a worktree against a base, or against HEAD. Returns {path, against, stat}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiffStat {
    /// The worktree's folder, inside workspace_root.
    #[schemars(length(min = 1))]
    pub path: String,
    /// What to diff against; absent, HEAD.
    #[schemars(length(min = 1))]
    pub base: Option<String>,
}

/// The worktree's branch, whether it is clean and its changed entries. Returns {path, branch, clean, entries}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Status {
    /// The worktree's folder, inside workspace_root.
    #[schemars(length(min = 1))]
    pub path: String,
}

fantastic_kernel::verbs! {
    "worktree_add" => WorktreeAdd,
    "worktree_remove" => WorktreeRemove,
    "commit" => Commit,
    "push" => Push,
    "fetch" => Fetch,
    "diff_stat" => DiffStat,
    "status" => Status,
}
