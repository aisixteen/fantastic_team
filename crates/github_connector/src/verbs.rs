//! The verbs the github connector declares: one struct per verb, then `verbs!` — the
//! list the agent's own entry checks every call against and `reflect` and `docs` render.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value};

/// Start with an empty call log (a fresh process or agent has made no calls).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Boot {}

/// Read this before using the connector. Returns {directive}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Directive {}

/// Who the credential is. Returns {app: {id, slug, name}} (app) or {user: {login, id}} (personal), plus repos, api_url.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Whoami {}

/// Push a work branch from a worktree of the record's git agent. Returns {repo, branch, path, pushed}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Push {
    /// The worktree's folder, inside the git agent's workspace_root.
    #[schemars(length(min = 1))]
    pub path: String,
    /// The work branch to push, a short name.
    #[schemars(length(min = 1))]
    pub branch: String,
    /// Never honoured: a force push is refused.
    pub force: Option<bool>,
}

/// Open a pull request, as a draft only. Returns {repo, number, url, draft, base}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrCreate {
    /// The repository, `owner/name`, on the connector's allowlist.
    #[schemars(length(min = 1))]
    pub repo: String,
    /// The work branch (never the default branch, `main` or `master`).
    #[schemars(length(min = 1))]
    pub branch: String,
    /// The pull request's title.
    #[schemars(length(min = 1))]
    pub title: String,
    /// The pull request's body.
    pub body: Option<String>,
    /// A lane's usage record, rendered as a footer under the body.
    pub usage: Option<Map<String, Value>>,
    /// Must be `true` when given: a person marks a pull request ready.
    pub draft: Option<bool>,
}

/// Which issues `issues` lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum StateFilter {
    /// Open ones.
    Open,
    /// Closed ones.
    Closed,
    /// Both.
    All,
}

impl StateFilter {
    /// The filter as GitHub's API spells it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            StateFilter::Open => "open",
            StateFilter::Closed => "closed",
            StateFilter::All => "all",
        }
    }
}

fn open_filter() -> StateFilter {
    StateFilter::Open
}

/// The issues of a repo (not its pull requests). Returns {repo, issues: [{number, title, state, labels}]}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Issues {
    /// The repository, `owner/name`, on the connector's allowlist.
    #[schemars(length(min = 1))]
    pub repo: String,
    /// Only issues with this label.
    #[schemars(length(min = 1))]
    pub label: Option<String>,
    /// Which issues.
    #[serde(default = "open_filter")]
    pub state: StateFilter,
}

/// One issue with its comments. Returns {repo, number, title, state, labels, body, comments}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueView {
    /// The repository, `owner/name`, on the connector's allowlist.
    #[schemars(length(min = 1))]
    pub repo: String,
    /// The issue's number.
    #[schemars(range(min = 1))]
    pub number: u64,
}

/// The labeled and unlabeled events of an issue. Returns {repo, number, events: [{event, label, actor, created_at}]}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LabelEvents {
    /// The repository, `owner/name`, on the connector's allowlist.
    #[schemars(length(min = 1))]
    pub repo: String,
    /// The issue's number.
    #[schemars(range(min = 1))]
    pub number: u64,
}

/// Comment on an issue or pull request. Returns {repo, number, comment_id, url}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Comment {
    /// The repository, `owner/name`, on the connector's allowlist.
    #[schemars(length(min = 1))]
    pub repo: String,
    /// The issue's or pull request's number.
    #[schemars(range(min = 1))]
    pub number: u64,
    /// The comment's text.
    #[schemars(length(min = 1))]
    pub body: String,
}

/// Add and/or remove labels. Give `add` and/or `remove`. Returns {repo, number, labels}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Label {
    /// The repository, `owner/name`, on the connector's allowlist.
    #[schemars(length(min = 1))]
    pub repo: String,
    /// The issue's or pull request's number.
    #[schemars(range(min = 1))]
    pub number: u64,
    /// Labels to add.
    #[schemars(length(min = 1), inner(length(min = 1)))]
    pub add: Option<Vec<String>>,
    /// Labels to remove.
    #[schemars(length(min = 1), inner(length(min = 1)))]
    pub remove: Option<Vec<String>>,
}

/// A pull request's state, checks and reviews. Returns {repo, number, state, draft, head_sha, checks, checks_summary, reviews, review_state}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrStatus {
    /// The repository, `owner/name`, on the connector's allowlist.
    #[schemars(length(min = 1))]
    pub repo: String,
    /// The pull request's number.
    #[schemars(range(min = 1))]
    pub number: u64,
}

/// How a pull request is merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MergeMethod {
    /// One squashed commit.
    Squash,
    /// A merge commit.
    Merge,
}

impl MergeMethod {
    /// The method as GitHub's API spells it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            MergeMethod::Squash => "squash",
            MergeMethod::Merge => "merge",
        }
    }
}

fn squash() -> MergeMethod {
    MergeMethod::Squash
}

/// Merge a green pull request, never by rebase: refuses unless it is open, targets the default branch and every check on the head commit passed; a draft is made ready first. Returns {merged, sha, number}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Merge {
    /// The repository, `owner/name`, on the connector's allowlist.
    #[schemars(length(min = 1))]
    pub repo: String,
    /// The pull request's number.
    #[schemars(range(min = 1))]
    pub number: u64,
    /// How to merge.
    #[serde(default = "squash")]
    pub method: MergeMethod,
}

/// One branch whose head is compared with a recorded commit.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StaleVersion {
    /// The repository, `owner/name`, on the connector's allowlist.
    #[schemars(length(min = 1))]
    pub repo: String,
    /// The branch.
    #[schemars(length(min = 1))]
    pub branch: String,
    /// The recorded commit; absent, the head only.
    #[schemars(length(min = 1))]
    pub commit: Option<String>,
}

/// One issue whose update time is compared with a recorded source version.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StaleIssue {
    /// The repository, `owner/name`, on the connector's allowlist.
    #[schemars(length(min = 1))]
    pub repo: String,
    /// The issue's number.
    #[schemars(range(min = 1))]
    pub number: u64,
    /// The recorded source version.
    #[schemars(length(min = 1))]
    pub source_version: Option<String>,
}

/// Facts for reusing a mark (no judgement): per repo the branch head and, given a recorded commit, the commits and files since it; for the issue, its `updated_at` and whether it moved since `source_version`. Give `versions` and/or `issue`. Returns {repos?: [{repo, branch, head, state: same|ahead|diverged|unknown, ahead_by, behind_by, commits: [{sha, subject}] (at most 50), files (at most 100), truncated}], issue?: {repo, number, updated_at, changed: bool|null}}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Staleness {
    /// The branches to compare.
    pub versions: Option<Vec<StaleVersion>>,
    /// The issue to compare.
    pub issue: Option<StaleIssue>,
}

/// Replace one line of an issue's body (a line outside a `## SPEC` section, matching `old` once). Returns {repo, number, line}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueEdit {
    /// The repository, `owner/name`, on the connector's allowlist.
    #[schemars(length(min = 1))]
    pub repo: String,
    /// The issue's number.
    #[schemars(range(min = 1))]
    pub number: u64,
    /// The line to replace (matched ignoring trailing space).
    #[schemars(length(min = 1))]
    pub old: String,
    /// What replaces it.
    pub new: String,
}

/// Open an issue as the app; with `project`, also add it to that board. Returns {repo, number, url, added_to_board}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueCreate {
    /// The repository, `owner/name`, on the connector's allowlist.
    #[schemars(length(min = 1))]
    pub repo: String,
    /// The issue's title.
    #[schemars(length(min = 1))]
    pub title: String,
    /// The issue's body.
    #[schemars(length(min = 1))]
    pub body: String,
    /// A ProjectV2 node id to add it to.
    #[schemars(length(min = 1))]
    pub project: Option<String>,
}

/// An issue's wanted state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum IssueStateWanted {
    /// Reopen.
    Open,
    /// Close.
    Closed,
}

impl IssueStateWanted {
    /// The state as GitHub's API spells it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            IssueStateWanted::Open => "open",
            IssueStateWanted::Closed => "closed",
        }
    }
}

/// Open or close an issue; closing an `epic` issue needs sender maintainer. Returns {repo, number, state}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueState {
    /// The repository, `owner/name`, on the connector's allowlist.
    #[schemars(length(min = 1))]
    pub repo: String,
    /// The issue's number.
    #[schemars(range(min = 1))]
    pub number: u64,
    /// The wanted state.
    pub state: IssueStateWanted,
    /// Who asked; `maintainer` may close an epic.
    pub sender: Option<String>,
}

/// Set one text field of a project item. Returns {repo, item, field}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectFieldSet {
    /// The repository, `owner/name`, on the connector's allowlist.
    #[schemars(length(min = 1))]
    pub repo: String,
    /// The ProjectV2 node id.
    #[schemars(length(min = 1))]
    pub project: String,
    /// The item's node id.
    #[schemars(length(min = 1))]
    pub item: String,
    /// The field's node id.
    #[schemars(length(min = 1))]
    pub field: String,
    /// The text to set.
    #[schemars(length(min = 1))]
    pub value: String,
}

/// Read one file through the Contents API. Returns {repo, path, ref?, text}, or {repo, path, not_found: true} when GitHub has no such file at that ref.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadFile {
    /// The repository, `owner/name`, on the connector's allowlist.
    #[schemars(length(min = 1))]
    pub repo: String,
    /// The file's path, relative, with no empty or `..` segment.
    #[schemars(length(min = 1))]
    pub path: String,
    /// A branch, tag or commit; absent, the default branch.
    #[schemars(length(min = 1))]
    pub r#ref: Option<String>,
}

fantastic_kernel::verbs! {
    "boot" => Boot,
    "directive" => Directive,
    "whoami" => Whoami,
    "push" => Push,
    "pr_create" => PrCreate,
    "issues" => Issues,
    "issue_create" => IssueCreate,
    "issue_state" => IssueState,
    "issue_view" => IssueView,
    "issue_edit" => IssueEdit,
    "label_events" => LabelEvents,
    "comment" => Comment,
    "label" => Label,
    "pr_status" => PrStatus,
    "merge" => Merge,
    "staleness" => Staleness,
    "project_field_set" => ProjectFieldSet,
    "read_file" => ReadFile,
}
