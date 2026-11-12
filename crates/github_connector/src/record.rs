//! The fields a github connector's record carries, beside the ones every record has.

use fantastic_ai_core::record::Credentials;
use schemars::JsonSchema;

/// The credential the connector acts with.
#[derive(Debug, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct App {
    /// The credential store and ref: the ref holds `token` (a personal token) or `app_id` + `private_key` (a GitHub App; PEM text or a path to the .pem), bound to api_url's host.
    pub credentials: Credentials,
}

/// A github connector's own record fields.
#[derive(Debug, JsonSchema)]
pub struct Record {
    /// The credential; no credential = local only.
    pub app: App,
    /// The allowlist: ["owner/name", …] — every other repo is refused.
    #[schemars(length(min = 1))]
    pub repos: Vec<String>,
    /// The GitHub API root (GitHub Enterprise: https://<host>/api/v3).
    #[serde(default = "api_url")]
    pub api_url: String,
    /// The id of a git connector agent (team_git.tools) whose worktrees push uses.
    #[schemars(length(min = 1), regex(pattern = r"\S"))]
    pub git: Option<String>,
    /// What the lead and workers may do with this connector, and how — returned by the directive verb.
    #[schemars(length(min = 1), regex(pattern = r"\S"))]
    pub directive: String,
    /// Seconds one GitHub API request may take.
    #[serde(default = "timeout_s")]
    #[schemars(range(min = 1))]
    pub timeout_s: u64,
}

fn api_url() -> String {
    "https://api.github.com".to_string()
}

fn timeout_s() -> u64 {
    30
}

fantastic_kernel::record!(Record);
