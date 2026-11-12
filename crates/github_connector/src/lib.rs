//! The github connector agent (`team_github.tools`): a personal token or a GitHub
//! App, bound by configuration. It pushes work branches through a git connector
//! agent, opens draft pull requests, and reads and writes issues, comments, labels
//! and PR status on an allowlist of repositories. Without a credential it is local
//! only: every GitHub verb says so.
//!
//! NO IDENTITY IN CODE: the token or app id and key, owner and repo names all come
//! from the record and the credential store. The mode is which fields the ref holds:
//! `token` (personal) or `app_id` + `private_key` (app).
//!
//! THE SECRET RULE: the credential is read from the credential store at each call.
//! In app mode a JWT and an installation token are minted per call. Neither they nor
//! a personal token live anywhere but local variables of that call, and never appear
//! in a reply, an error, a log line, a record, a payload sent to another agent, argv,
//! a URL or a file. Every error string passes through `redact_all` before it is returned.
#![deny(missing_docs)]

mod api;
mod app;
mod callog;
mod footer;
mod calls;
mod push;
pub mod record;
pub mod verbs;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_auth;
#[cfg(test)]
mod tests_read_file;
#[cfg(test)]
mod tests_secrets;
#[cfg(test)]
mod tests_staleness;

use std::sync::Arc;

use async_trait::async_trait;
use fantastic_ai_core::helpers::agent_meta;
use fantastic_kernel::bundle::{BundleError, Handler, Reply};
use fantastic_kernel::refusal::{refusal, with, BAD_ARG};
use fantastic_kernel::{AgentId, Kernel};
use serde_json::{Map, Value};

/// The handler module name this bundle registers under.
pub const HANDLER_MODULE: &str = "team_github.tools";
/// The bundle name.
const NAME: &str = "team_github";
/// What the bundle is for: its one short man text; how to call it is rendered from its
/// declared verbs (`docs`, `reflect`).
pub const MAN: &str = "github connector — a personal token or a GitHub App, bound by configuration: push via a git agent, draft pull requests, issues, comments, labels and PR status on allowlisted repos; local only without a credential.";

/// The github connector bundle.
#[derive(Default)]
pub struct GithubConnectorBundle;

/// The config of the agent, read from its meta at every call.
pub(crate) struct Cfg {
    /// The credential store and ref of the app's key.
    pub app_cred: (String, String),
    /// The allowlist of `owner/name` repositories.
    pub repos: Vec<String>,
    /// The API root, without a trailing slash.
    pub api_url: String,
    /// The id of the git connector agent push uses.
    pub git: Option<String>,
    /// What the connector may be used for.
    pub directive: String,
    /// Seconds one request may take.
    pub timeout_s: u64,
}

/// True for `owner/name` where both parts are plain names.
pub(crate) fn valid_repo(s: &str) -> bool {
    let mut parts = s.split('/');
    let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let ok = |p: &str| {
        !p.is_empty()
            && p != "."
            && p != ".."
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    ok(owner) && ok(name)
}

/// The text of record field `key`, as the declaration reads it.
fn field_text(meta: &Map<String, Value>, key: &str) -> Option<String> {
    record::RECORD
        .get(meta, key)
        .and_then(|v| v.as_str().map(str::to_string))
}

impl GithubConnectorBundle {
    /// Read the config from the agent's record.
    pub(crate) fn cfg(meta: &Map<String, Value>) -> Cfg {
        let cred = meta.get("app").and_then(|a| a.get("credentials"));
        let app_cred = (
            cred.and_then(|c| c["store"].as_str())
                .unwrap_or("")
                .to_string(),
            cred.and_then(|c| c["ref"].as_str())
                .unwrap_or("")
                .to_string(),
        );
        Cfg {
            app_cred,
            repos: meta
                .get("repos")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            api_url: field_text(meta, "api_url")
                .unwrap_or_default()
                .trim_end_matches('/')
                .to_string(),
            git: field_text(meta, "git").filter(|s| !s.is_empty()),
            directive: field_text(meta, "directive").unwrap_or_default(),
            timeout_s: record::RECORD
                .get(meta, "timeout_s")
                .and_then(|v| v.as_u64())
                .unwrap_or_default(),
        }
    }
}

/// A refusal reply: `error` text plus the arg that caused it.
pub(crate) fn bad(text: String, arg: &str, value: Value) -> Value {
    let mut r = refusal(BAD_ARG, text);
    with(&mut r, arg, value);
    r
}

fn validate_repos(repos: &[Value]) -> Option<String> {
    if repos.iter().all(|r| r.as_str().is_some_and(valid_repo)) {
        None
    } else {
        Some("team_github: `repos` must be a non-empty list of \"owner/name\"".into())
    }
}

fn validate_api_url(v: &Value) -> Option<String> {
    let bad = || {
        Some(
            "team_github: `api_url` must be an https URL (or http to 127.0.0.1, ::1 or localhost)"
                .into(),
        )
    };
    let Some(s) = v.as_str() else { return bad() };
    let Ok(u) = reqwest::Url::parse(s) else {
        return bad();
    };
    let host = u.host_str().unwrap_or("");
    let local = matches!(host, "127.0.0.1" | "::1" | "[::1]" | "localhost");
    let scheme_ok = u.scheme() == "https" || (u.scheme() == "http" && local);
    if scheme_ok && !host.is_empty() && u.query().is_none() && u.fragment().is_none() {
        None
    } else {
        bad()
    }
}

#[async_trait]
impl Handler for GithubConnectorBundle {
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

    /// What the record's declaration cannot say: each repo is `owner/name` and the API
    /// root is an https URL (or http to loopback).
    fn validate_meta(
        &self,
        meta: &Map<String, Value>,
        _record: &Map<String, Value>,
    ) -> Option<String> {
        if let Some(repos) = meta.get("repos").and_then(Value::as_array) {
            if let Some(e) = validate_repos(repos) {
                return Some(e);
            }
        }
        if let Some(v) = meta.get("api_url").filter(|v| !v.is_null()) {
            if let Some(e) = validate_api_url(v) {
                return Some(e);
            }
        }
        None
    }

    async fn handle(
        &self,
        agent_id: &AgentId,
        call: Self::Call,
        kernel: &Arc<Kernel>,
    ) -> Result<Reply, BundleError> {
        let cfg = Self::cfg(&agent_meta(agent_id, kernel));
        let out = calls::dispatch(call, agent_id, kernel, &cfg).await;
        Ok(Some(out))
    }
}
