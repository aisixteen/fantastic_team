//! The git connector agent (`team_git.tools`): one local clone and one
//! remote. It makes worktrees, commits and pushes work branches. It never
//! pushes the default branch, `main` or `master`, and never forces.
//!
//! THE SECRET RULE: an HTTPS token is read from the credential store at each
//! fetch or push and handed to git only through the child's environment and a
//! one-shot credential helper. It is never in argv, a URL, a git config, a
//! reply, an error or a log line. Every string returned passes through `redact`.
#![deny(missing_docs)]

pub mod git;
pub mod paths;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_secrets;

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use fantastic_ai_core::helpers::agent_meta;
use fantastic_kernel::bundle::{BundleError, Handler, Reply};
use fantastic_kernel::refusal::{refusal, with, BAD_ARG};
use fantastic_kernel::{AgentId, Kernel};
use serde_json::{json, Map, Value};

pub mod record;
pub mod verbs;

/// The handler module name this bundle registers under.
pub const HANDLER_MODULE: &str = "team_git.tools";
/// The bundle name.
const NAME: &str = "team_git";
/// What the bundle is for: its one short man text; how to call it is rendered from its
/// declared verbs (`docs`, `reflect`).
pub const MAN: &str = "git connector — worktrees, commits and pushes of work branches to one remote; never the default branch, never a force. One agent is one local clone and one remote (SSH, HTTPS or a local path); an HTTPS token is read from the credential store at each fetch and push and never leaves the agent.";

/// Refusal text: the record's remote carries a user or password.
pub(crate) const URL_CREDS: &str = "team_git: the remote URL carries a user or password — remove it and name a credential store ref in credentials instead";

/// The git connector bundle.
#[derive(Default)]
pub struct GitConnectorBundle;

/// The config of the agent, read from its meta at every call.
pub struct Cfg {
    /// Absolute path of the local clone every worktree is made from.
    pub repo_dir: PathBuf,
    /// Absolute directory every worktree path must be inside.
    pub workspace_root: PathBuf,
    /// The record names no root: `workspace_root` is the default one, made when a worktree needs it.
    pub default_root: bool,
    /// A remote name in `repo_dir`, or a URL.
    pub remote: String,
    /// The remote's default branch; push refuses it.
    pub default_branch: String,
    /// The HTTPS user name sent with a token.
    pub username: String,
    /// Commit author and sign-off `(name, email)`, when both are set.
    pub author: Option<(String, String)>,
    /// GIT_SSH_COMMAND for SSH remotes.
    pub ssh_command: String,
    /// Seconds one git command may run.
    pub timeout_s: u64,
    /// Whether the record names a credential store ref.
    pub has_credentials: bool,
}

/// The text of record field `key`, as the declaration reads it (the record's own value, else the declared default).
fn field_text(meta: &Map<String, Value>, key: &str) -> Option<String> {
    record::RECORD
        .get(meta, key)
        .and_then(|v| v.as_str().map(str::to_string))
}

impl GitConnectorBundle {
    /// Read the config of agent `id` from its record.
    pub(crate) fn cfg(id: &str, meta: &Map<String, Value>) -> Cfg {
        let root = field_text(meta, "workspace_root").filter(|r| !r.is_empty());
        Cfg {
            repo_dir: PathBuf::from(field_text(meta, "repo_dir").unwrap_or_default()),
            default_root: root.is_none(),
            workspace_root: root
                .map(PathBuf::from)
                .unwrap_or_else(|| default_workspace_root(id)),
            remote: field_text(meta, "remote").unwrap_or_default(),
            default_branch: field_text(meta, "default_branch").unwrap_or_default(),
            username: field_text(meta, "username").unwrap_or_default(),
            author: match (field_text(meta, "author_name"), field_text(meta, "author_email")) {
                (Some(n), Some(e)) => Some((n, e)),
                _ => None,
            },
            ssh_command: field_text(meta, "ssh_command").unwrap_or_default(),
            timeout_s: record::RECORD
                .get(meta, "timeout_s")
                .and_then(|v| v.as_u64())
                .unwrap_or_default(),
            has_credentials: meta
                .get("credentials")
                .map(|c| !c.is_null())
                .unwrap_or(false),
        }
    }
}

/// The default worktree root of git agent `id`: `<worktrees folder>/<id>`, the folder being
/// `<system temp>/fantastic_team/worktrees`.
pub fn default_workspace_root(id: &str) -> PathBuf {
    std::env::temp_dir()
        .join("fantastic_team")
        .join("worktrees")
        .join(id)
}

/// The config of the git agent `id`, read from its record the way `handle` reads it.
pub fn cfg_of(kernel: &Kernel, id: &AgentId) -> Result<Cfg, Value> {
    let entry = kernel
        .agents
        .get(id)
        .ok_or_else(|| json!({"error": format!("team_git: no agent `{id}`")}))?;
    let meta = entry.meta.read().expect("meta").clone();
    Ok(GitConnectorBundle::cfg(id.as_str(), &meta))
}

/// A refusal reply: `error` text plus the arg that caused it.
pub(crate) fn bad(text: String, arg: &str, value: Value) -> Value {
    let mut r = refusal(BAD_ARG, text);
    with(&mut r, arg, value);
    r
}

#[async_trait]
impl Handler for GitConnectorBundle {
    type Call = verbs::Call;

    fn name(&self) -> &str {
        NAME
    }

    fn man(&self) -> &'static str {
        MAN
    }

    fn verbs(&self) -> &'static [fantastic_kernel::Verb] {
        verbs::VERBS
    }

    fn dynamic_verbs(&self, agent: &fantastic_kernel::Agent) -> Vec<fantastic_kernel::DynVerb> {
        verbs::dynamic_verbs(agent)
    }

    fn record(&self) -> &'static fantastic_kernel::RecordDecl {
        &record::RECORD
    }

    /// What the record's declaration cannot say: the paths are absolute, the remote
    /// carries no user or password, and the author is both name and email or neither.
    fn validate_meta(
        &self,
        meta: &Map<String, Value>,
        record: &Map<String, Value>,
    ) -> Option<String> {
        for key in ["repo_dir", "workspace_root"] {
            if let Some(s) = meta.get(key).and_then(|v| v.as_str()) {
                if !std::path::Path::new(s).is_absolute() {
                    return Some(format!("team_git: `{key}` must be an absolute path"));
                }
            }
        }
        if let Some(s) = meta.get("remote").and_then(|v| v.as_str()) {
            if s.is_empty() || s.starts_with('-') {
                return Some("team_git: `remote` must be a remote name or a URL".into());
            }
            let lower = s.to_ascii_lowercase();
            if lower.starts_with("http://") || lower.starts_with("https://") {
                let rest = &s[s.find("//").unwrap() + 2..];
                let authority = rest.split('/').next().unwrap_or("");
                if authority.contains('@') {
                    return Some(URL_CREDS.to_string());
                }
            }
        }
        if let Some(s) = meta.get("default_branch").and_then(|v| v.as_str()) {
            if s.is_empty() || s.starts_with('-') {
                return Some("team_git: `default_branch` must be a branch name".into());
            }
        }
        for key in ["username", "ssh_command"] {
            if meta.get(key).and_then(|v| v.as_str()) == Some("") {
                return Some(format!("team_git: `{key}` must be a non-empty string"));
            }
        }
        // Checked on the merged record: the record plus the write.
        let set = |k: &str| {
            meta.get(k).filter(|v| !v.is_null()).is_some()
                || record.get(k).filter(|v| !v.is_null()).is_some()
        };
        if set("author_name") != set("author_email") {
            return Some(
                "team_git: `author_name` and `author_email` go together — set both or neither"
                    .into(),
            );
        }
        None
    }

    async fn handle(
        &self,
        agent_id: &AgentId,
        call: Self::Call,
        kernel: &Arc<Kernel>,
    ) -> Result<Reply, BundleError> {
        let cfg = Self::cfg(agent_id.as_str(), &agent_meta(agent_id, kernel));
        let out: Value = match call {
            verbs::Call::Reflect(_) => json!({
                "id": agent_id.to_string(),
                "sentence": MAN,
                "repo_dir": cfg.repo_dir,
                "workspace_root": cfg.workspace_root,
                "remote": cfg.remote,
                "default_branch": cfg.default_branch,
                "credentials": cfg.has_credentials,
            }),
            verbs::Call::WorktreeAdd(a) => match git::worktree_add(&cfg, &a).await {
                Ok(v) | Err(v) => v,
            },
            verbs::Call::WorktreeRemove(a) => match git::worktree_remove(&cfg, &a).await {
                Ok(v) | Err(v) => v,
            },
            verbs::Call::Commit(a) => match git::commit(&cfg, &a).await {
                Ok(v) | Err(v) => v,
            },
            verbs::Call::Push(a) => match git::push(&cfg, &a, agent_id, kernel).await {
                Ok(v) | Err(v) => v,
            },
            verbs::Call::Fetch(_) => match git::fetch(&cfg, agent_id, kernel).await {
                Ok(v) | Err(v) => v,
            },
            verbs::Call::DiffStat(a) => match git::diff_stat(&cfg, &a).await {
                Ok(v) | Err(v) => v,
            },
            verbs::Call::Status(a) => match git::status(&cfg, &a).await {
                Ok(v) | Err(v) => v,
            },
        };
        Ok(Some(out))
    }
}
