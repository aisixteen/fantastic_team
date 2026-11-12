//! The fields a puller's record carries, beside the ones every record has.

use schemars::JsonSchema;
use serde_json::{Map, Value};

/// A puller's own record fields.
#[derive(Debug, JsonSchema)]
pub struct Record {
    /// The repositories to pull accepted issues from, `owner/name`.
    pub repos: Vec<String>,
    /// The maintainer's login: the only actor who may apply `team-ready`.
    #[schemars(length(min = 1), regex(pattern = r"\S"))]
    pub maintainer: String,
    /// The app's login, whose `## SPEC` comments count beside the maintainer's.
    #[schemars(length(min = 1), regex(pattern = r"\S"))]
    pub app: String,
    /// The github connector agent the issues are read through.
    #[schemars(length(min = 1), regex(pattern = r"\S"))]
    pub github: String,
    /// The store agent the queue files are written through.
    #[schemars(length(min = 1), regex(pattern = r"\S"))]
    pub store: String,
    /// Per-repo settings, by `owner/name`; a repo with no entry takes the defaults.
    pub repo_config: Option<Map<String, Value>>,
}

fantastic_kernel::record!(Record);
