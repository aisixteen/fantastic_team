//! The GitHub credential, read from the credential store at each call. The mode is
//! which fields the ref holds: `token` (personal, used as is) or `app_id` +
//! `private_key` (a GitHub App: a JWT and an installation token are minted per call,
//! live only in that call's locals, and are never cached, returned, stored or
//! logged). No credential means local only.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use fantastic_kernel::refusal::{refusal, NOT_FOUND};
use fantastic_kernel::{AgentId, Item, Kernel, Status};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::Method;
use serde_json::{json, Value};

use crate::api::Api;
use crate::Cfg;

/// The answer of every GitHub verb when no credential is defined.
pub(crate) const LOCAL_ONLY: &str =
    "no GitHub credential defined: local work only (set `GH_TOKEN`, or an app id + key)";

/// The app id and private key read from the store. No Debug, Display or Serialize.
pub(crate) struct AppKey {
    pub app_id: String,
    pub pem: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Claims {
    pub iat: u64,
    pub exp: u64,
    pub iss: String,
}

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub(crate) fn cred_error(cfg: &Cfg, what: &str) -> Value {
    json!({"error": format!(
        "team_github: credential `{}` in `{}`: {what}",
        cfg.app_cred.1, cfg.app_cred.0
    )})
}

/// What a ref in the store holds. No Debug, Display or Serialize.
pub(crate) enum Cred {
    /// A personal token, used as is.
    Token(String),
    /// A GitHub App: a JWT is signed and a per-call installation token minted.
    App(AppKey),
}

/// Why no credential could be read.
pub(crate) enum CredErr {
    /// Nothing is defined: GitHub work is off.
    LocalOnly,
    /// A defined credential that cannot be used; the reply names the ref and store.
    Bad(Value),
}

impl CredErr {
    /// The reply a verb answers with.
    pub(crate) fn reply(self) -> Value {
        match self {
            CredErr::LocalOnly => refusal(NOT_FOUND, LOCAL_ONLY),
            CredErr::Bad(v) => v,
        }
    }
}

fn present(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Number(n)) if n.as_u64().is_some_and(|v| v > 0) => Some(n.to_string()),
        _ => None,
    }
}

fn pem_of(raw: &str) -> Option<String> {
    if raw.trim_start().starts_with("-----BEGIN") {
        return Some(raw.to_string());
    }
    let path = match raw.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", std::env::var("HOME").ok()?),
        None => raw.to_string(),
    };
    let text = std::fs::read_to_string(path).ok()?;
    text.contains("-----BEGIN").then_some(text)
}

/// Read the credential from the store, sent as this agent. The mode is which fields
/// the ref holds. Errors name the ref and store, never a value.
pub(crate) async fn read_cred(kernel: &Arc<Kernel>, cfg: &Cfg) -> Result<Cred, CredErr> {
    let (store, r) = cfg.app_cred.clone();
    let outs = kernel
        .send(vec![Item::new(
            store.as_str(),
            json!({"type": "get", "ref": r, "host": cfg.api_url}),
        )])
        .await;
    let Some(out) = outs.into_iter().next() else {
        return Err(CredErr::LocalOnly);
    };
    if !matches!(out.status, Status::Completed) || !out.result.is_object() {
        return Err(CredErr::LocalOnly);
    }
    if matches!(
        out.result.get("reason").and_then(|v| v.as_str()),
        Some("not_found" | "unknown_agent")
    ) {
        return Err(CredErr::LocalOnly);
    }
    if out.result.get("error").is_some_and(|v| match v {
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Null => false,
        _ => true,
    }) {
        return Err(CredErr::Bad(cred_error(cfg, "the store refused the read")));
    }
    let fields = out.result.get("fields").cloned().unwrap_or(Value::Null);
    let token = present(fields.get("token"));
    let app_id = present(fields.get("app_id"));
    let key = present(fields.get("private_key"));
    let bad = |what: &str| Err(CredErr::Bad(cred_error(cfg, what)));
    match (token, app_id, key) {
        (Some(t), None, None) => Ok(Cred::Token(t)),
        (None, Some(app_id), Some(raw)) => match pem_of(&raw) {
            Some(pem) => Ok(Cred::App(AppKey { app_id, pem })),
            None => bad("`private_key` is neither PEM text nor a readable .pem file"),
        },
        (Some(_), _, _) => bad(
            "holds `token` and app fields — keep one mode: `token` (personal) or `app_id` + `private_key` (app)",
        ),
        (None, Some(_), None) => bad("has `app_id` but no `private_key` — app mode needs both"),
        (None, None, Some(_)) => bad("has `private_key` but no `app_id` — app mode needs both"),
        (None, None, None) => bad(
            "has no `token` (personal) and no `app_id` + `private_key` (app)",
        ),
    }
}

/// Sign an RS256 JWT for the app: issued 60 s back, valid 10 minutes at most.
pub(crate) fn sign_jwt(app_id: &str, pem: &str, now: u64) -> Result<String, String> {
    let claims = Claims {
        iat: now.saturating_sub(60),
        exp: now + 540,
        iss: app_id.to_string(),
    };
    let key = EncodingKey::from_rsa_pem(pem.as_bytes())
        .map_err(|_| "private_key is not an RSA private key in PEM form".to_string())?;
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key)
        .map_err(|_| "could not sign the JWT".to_string())
}

/// The token for one call: a personal token as is, or an installation token scoped
/// to the one repo, minted for this call only.
pub(crate) async fn token_for(
    id: &AgentId,
    kernel: &Arc<Kernel>,
    cfg: &Cfg,
    api: &Api,
    verb: &str,
    repo: &str,
) -> Result<String, Value> {
    let _ = id;
    let key = match read_cred(kernel, cfg).await.map_err(CredErr::reply)? {
        Cred::Token(t) => return Ok(t),
        Cred::App(k) => k,
    };
    let jwt = sign_jwt(&key.app_id, &key.pem, now_secs()).map_err(|e| cred_error(cfg, &e))?;
    let jw = jwt.as_str();
    let inst = api
        .call(
            verb,
            Method::GET,
            &format!("/repos/{repo}/installation"),
            &[],
            jw,
            None,
        )
        .await
        .map_err(|(_, v)| v)?;
    let install = inst.get("id").and_then(|v| v.as_u64()).ok_or_else(
        || json!({"error": format!("team_github {verb}: no installation id for {repo}")}),
    )?;
    let name = repo.split('/').nth(1).unwrap_or("");
    let minted = api
        .call(
            verb,
            Method::POST,
            &format!("/app/installations/{install}/access_tokens"),
            &[],
            jw,
            Some(&json!({"repositories": [name]})),
        )
        .await
        .map_err(|(_, v)| v)?;
    minted
        .get("token")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .ok_or_else(|| json!({"error": format!("team_github {verb}: GitHub minted no token")}))
}

/// The auth mode for `reflect` and the view. Values are dropped at once.
pub(crate) async fn auth_state(kernel: &Arc<Kernel>, cfg: &Cfg) -> Value {
    match read_cred(kernel, cfg).await {
        Ok(Cred::Token(_)) => json!({"mode": "personal"}),
        Ok(Cred::App(_)) => json!({"mode": "app"}),
        Err(CredErr::LocalOnly) => json!({"mode": "local", "note": LOCAL_ONLY}),
        Err(CredErr::Bad(v)) => json!({"mode": "invalid", "note": v["error"]}),
    }
}
