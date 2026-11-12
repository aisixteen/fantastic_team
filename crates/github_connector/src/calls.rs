//! The verbs. Each API verb checks its arguments and the repo allowlist first,
//! then mints a token for that one repo, then makes its requests. Every reply is
//! scrubbed of the token before it is returned.

use std::sync::Arc;

use fantastic_kernel::refusal::missing;
use fantastic_kernel::{AgentId, Kernel};
use reqwest::Method;
use serde_json::{json, Value};

use team_git_connector::git::scrub_all;

use crate::api::Api;
use crate::verbs::{self, Call};
use crate::{app, bad, push, Cfg, MAN};

const LABEL_TEXT: &str =
    "team_github label: give `add` and/or `remove` — each a non-empty list of label names";
const DRAFT_TEXT: &str =
    "team_github pr_create: pull requests are opened as drafts only — a person marks one ready";

/// Route one call to its handler, and log it for `reflect`.
pub(crate) async fn dispatch(call: Call, id: &AgentId, kernel: &Arc<Kernel>, cfg: &Cfg) -> Value {
    let logged = !matches!(call, Call::Reflect(_) | Call::Directive(_) | Call::Boot(_));
    let (verb, target) = (verb_of(&call), crate::callog::target_of(&call));
    let reply = route(call, id, kernel, cfg).await;
    if logged {
        crate::callog::push(
            &id.to_string(),
            &cfg.api_url,
            verb,
            target,
            crate::callog::result_of(&reply),
        );
    }
    reply
}

/// The verb a call was made with, as the call log names it.
fn verb_of(call: &Call) -> &'static str {
    match call {
        Call::Reflect(_) => "reflect",
        Call::Boot(_) => "boot",
        Call::Directive(_) => "directive",
        Call::Whoami(_) => "whoami",
        Call::Push(_) => "push",
        Call::PrCreate(_) => "pr_create",
        Call::Issues(_) => "issues",
        Call::IssueCreate(_) => "issue_create",
        Call::IssueState(_) => "issue_state",
        Call::IssueView(_) => "issue_view",
        Call::IssueEdit(_) => "issue_edit",
        Call::LabelEvents(_) => "label_events",
        Call::Comment(_) => "comment",
        Call::Label(_) => "label",
        Call::PrStatus(_) => "pr_status",
        Call::Merge(_) => "merge",
        Call::Staleness(_) => "staleness",
        Call::ProjectFieldSet(_) => "project_field_set",
        Call::ReadFile(_) => "read_file",
    }
}

async fn route(call: Call, id: &AgentId, kernel: &Arc<Kernel>, cfg: &Cfg) -> Value {
    match call {
        Call::Reflect(_) => {
            let auth = app::auth_state(kernel, cfg).await;
            json!({
                "id": id.to_string(),
                "sentence": MAN,
                "repos": cfg.repos,
                "api_url": cfg.api_url,
                "git": cfg.git,
                "directive": cfg.directive,
                "recent_calls": crate::callog::recent(&id.to_string(), &cfg.api_url),
                "rate": crate::callog::rate(&cfg.api_url),
                "auth": auth,
            })
        }
        Call::Boot(_) => {
            // A fresh process or agent starts with an empty call log.
            crate::callog::reset(&id.to_string(), &cfg.api_url);
            Value::Null
        }
        Call::Directive(_) => json!({"directive": cfg.directive}),
        Call::Whoami(_) => whoami(id, kernel, cfg).await,
        Call::Push(a) => push::push(id, kernel, cfg, &a).await,
        Call::PrCreate(a) => pr_create(id, kernel, cfg, &a).await,
        Call::Merge(a) => merge(id, kernel, cfg, &a).await,
        Call::Issues(a) => issues(id, kernel, cfg, &a).await,
        Call::IssueView(a) => issue_view(id, kernel, cfg, &a).await,
        Call::LabelEvents(a) => label_events(id, kernel, cfg, &a).await,
        Call::Comment(a) => comment(id, kernel, cfg, &a).await,
        Call::Label(a) => label(id, kernel, cfg, &a).await,
        Call::PrStatus(a) => pr_status(id, kernel, cfg, &a).await,
        Call::Staleness(a) => staleness(id, kernel, cfg, &a).await,
        Call::IssueEdit(a) => issue_edit(id, kernel, cfg, &a).await,
        Call::IssueCreate(a) => issue_create(id, kernel, cfg, &a).await,
        Call::IssueState(a) => issue_state(id, kernel, cfg, &a).await,
        Call::ProjectFieldSet(a) => project_field_set(id, kernel, cfg, &a).await,
        Call::ReadFile(a) => read_file(id, kernel, cfg, &a).await,
    }
}

/// Read one file through the Contents API. Returns `{repo, path, ref?, text}`,
/// or `{repo, path, not_found: true}` when GitHub has no such file at that ref.
async fn read_file(id: &AgentId, kernel: &Arc<Kernel>, cfg: &Cfg, a: &verbs::ReadFile) -> Value {
    const V: &str = "read_file";
    let repo = match repo_arg(&a.repo, V, cfg) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let path = a.path.clone();
    if path.starts_with('/') || path.split('/').any(|seg| seg.is_empty() || seg == "..") {
        return bad(
            "team_github read_file: `path` must be relative, with no empty or `..` segments".into(),
            "path",
            json!(path),
        );
    }
    let git_ref = a.r#ref.clone();
    let api = Api::new(cfg);
    let token = match app::token_for(id, kernel, cfg, &api, V, &repo).await {
        Ok(t) => t,
        Err(e) => return e,
    };
    let encoded: String = path.split('/').map(pct).collect::<Vec<_>>().join("/");
    let mut q: Vec<(&str, String)> = Vec::new();
    if let Some(r) = &git_ref {
        q.push(("ref", r.clone()));
    }
    let r: Result<Value, Value> = async {
        match api
            .call(
                V,
                Method::GET,
                &format!("/repos/{repo}/contents/{encoded}"),
                &q,
                &token,
                None,
            )
            .await
        {
            Ok(v) => {
                let raw = v["content"].as_str().unwrap_or("");
                let cleaned: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
                let bytes = match v["encoding"].as_str() {
                    Some("base64") => {
                        use base64::Engine;
                        base64::engine::general_purpose::STANDARD
                            .decode(cleaned.as_bytes())
                            .map_err(|_| {
                                bad(
                                    format!("team_github {V}: `{path}` is not valid base64 in GitHub's answer"),
                                    "path",
                                    json!(path),
                                )
                            })?
                    }
                    _ => return Err(bad(
                        format!("team_github {V}: `{path}` has no base64 content (a directory or a link?)"),
                        "path",
                        json!(path),
                    )),
                };
                let text = String::from_utf8(bytes).map_err(|_| {
                    bad(
                        format!("team_github {V}: `{path}` is not UTF-8 text"),
                        "path",
                        json!(path),
                    )
                })?;
                let mut out = json!({"repo": repo, "path": path, "text": text});
                if let Some(r) = &git_ref {
                    out["ref"] = json!(r);
                }
                Ok(out)
            }
            Err((404, _)) => Ok(json!({"repo": repo, "path": path, "not_found": true})),
            Err((_, v)) => Err(v),
        }
    }
    .await;
    finish(r, &token)
}

fn ge(r: Result<Value, (u16, Value)>) -> Result<Value, Value> {
    r.map_err(|(_, v)| v)
}

fn repo_arg(raw: &str, verb: &str, cfg: &Cfg) -> Result<String, Value> {
    match cfg.repos.iter().find(|r| r.eq_ignore_ascii_case(raw)) {
        Some(r) => Ok(r.clone()),
        None => Err(bad(
            format!(
                "team_github {verb}: `{raw}` is not in this connector's repos ({})",
                cfg.repos.join(", ")
            ),
            "repo",
            json!(raw),
        )),
    }
}

fn names_of(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|l| l["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Percent-encode a label name for a path segment.
fn pct(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn finish(r: Result<Value, Value>, token: &str) -> Value {
    let mut v = match r {
        Ok(v) | Err(v) => v,
    };
    scrub_all(&mut v, &[token]);
    v
}

fn base_refusal(branch: &str) -> Value {
    bad(
        format!("team_github pr_create: `{branch}` is a base branch (the default branch, main or master) — open the pull request from a work branch"),
        "branch",
        json!(branch),
    )
}

async fn whoami(_id: &AgentId, kernel: &Arc<Kernel>, cfg: &Cfg) -> Value {
    let cred = match app::read_cred(kernel, cfg).await {
        Ok(c) => c,
        Err(e) => return e.reply(),
    };
    let api = Api::new(cfg);
    match cred {
        app::Cred::App(key) => {
            let jwt = match app::sign_jwt(&key.app_id, &key.pem, app::now_secs()) {
                Ok(j) => j,
                Err(e) => return app::cred_error(cfg, &e),
            };
            let r: Result<Value, Value> = async {
                let app = ge(api
                    .call("whoami", Method::GET, "/app", &[], &jwt, None)
                    .await)?;
                Ok(json!({
                    "app": {"id": app["id"], "slug": app["slug"], "name": app["name"]},
                    "repos": cfg.repos,
                    "api_url": cfg.api_url,
                }))
            }
            .await;
            finish(r, &jwt)
        }
        app::Cred::Token(t) => {
            let r: Result<Value, Value> = async {
                let u = ge(api
                    .call("whoami", Method::GET, "/user", &[], &t, None)
                    .await)?;
                Ok(json!({
                    "user": {"login": u["login"], "id": u["id"]},
                    "repos": cfg.repos,
                    "api_url": cfg.api_url,
                }))
            }
            .await;
            finish(r, &t)
        }
    }
}

async fn pr_create(id: &AgentId, kernel: &Arc<Kernel>, cfg: &Cfg, a: &verbs::PrCreate) -> Value {
    const V: &str = "pr_create";
    if a.draft == Some(false) {
        return bad(DRAFT_TEXT.into(), "draft", json!(false));
    }
    let branch = a.branch.as_str();
    if branch == "main" || branch == "master" {
        return base_refusal(branch);
    }
    let title = a.title.clone();
    let repo = match repo_arg(&a.repo, V, cfg) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let api = Api::new(cfg);
    let token = match app::token_for(id, kernel, cfg, &api, V, &repo).await {
        Ok(t) => t,
        Err(e) => return e,
    };
    let r: Result<Value, Value> = async {
        let info = ge(api
            .call(V, Method::GET, &format!("/repos/{repo}"), &[], &token, None)
            .await)?;
        let default = info["default_branch"].as_str().unwrap_or("").to_string();
        if branch == default {
            return Err(base_refusal(branch));
        }
        let body_text = crate::footer::with_footer(
            a.body.as_deref().unwrap_or(""),
            a.usage.clone().map(Value::Object).as_ref(),
        );
        let payload = json!({
            "title": title,
            "head": branch,
            "base": default,
            "body": body_text,
            "draft": true,
        });
        let pr = ge(api
            .call(
                V,
                Method::POST,
                &format!("/repos/{repo}/pulls"),
                &[],
                &token,
                Some(&payload),
            )
            .await)?;
        Ok(json!({
            "repo": repo,
            "number": pr["number"],
            "url": pr["html_url"],
            "draft": pr["draft"],
            "base": default,
        }))
    }
    .await;
    finish(r, &token)
}

async fn issues(id: &AgentId, kernel: &Arc<Kernel>, cfg: &Cfg, a: &verbs::Issues) -> Value {
    const V: &str = "issues";
    let repo = match repo_arg(&a.repo, V, cfg) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let state = a.state.as_str().to_string();
    let label = a.label.clone();
    let api = Api::new(cfg);
    let token = match app::token_for(id, kernel, cfg, &api, V, &repo).await {
        Ok(t) => t,
        Err(e) => return e,
    };
    let r: Result<Value, Value> = async {
        let mut q: Vec<(&str, String)> = vec![("state", state.clone()), ("per_page", "100".into())];
        if let Some(l) = &label {
            q.push(("labels", l.clone()));
        }
        let items = ge(api
            .call(
                V,
                Method::GET,
                &format!("/repos/{repo}/issues"),
                &q,
                &token,
                None,
            )
            .await)?;
        let list: Vec<Value> = items
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|i| i.get("pull_request").is_none())
                    .map(|i| {
                        json!({
                            "number": i["number"],
                            "title": i["title"],
                            "state": i["state"],
                            "labels": names_of(&i["labels"]),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(json!({"repo": repo, "issues": list}))
    }
    .await;
    finish(r, &token)
}

async fn issue_view(id: &AgentId, kernel: &Arc<Kernel>, cfg: &Cfg, a: &verbs::IssueView) -> Value {
    const V: &str = "issue_view";
    let repo = match repo_arg(&a.repo, V, cfg) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let number = a.number;
    let api = Api::new(cfg);
    let token = match app::token_for(id, kernel, cfg, &api, V, &repo).await {
        Ok(t) => t,
        Err(e) => return e,
    };
    let r: Result<Value, Value> = async {
        let issue = ge(api
            .call(
                V,
                Method::GET,
                &format!("/repos/{repo}/issues/{number}"),
                &[],
                &token,
                None,
            )
            .await)?;
        let comments = ge(api
            .call(
                V,
                Method::GET,
                &format!("/repos/{repo}/issues/{number}/comments"),
                &[("per_page", "100".into())],
                &token,
                None,
            )
            .await)?;
        let list: Vec<Value> = comments
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|c| {
                        json!({
                            "author": c["user"]["login"],
                            "created_at": c["created_at"],
                            "body": c["body"],
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(json!({
            "repo": repo,
            "number": number,
            "title": issue["title"],
            "state": issue["state"],
            "labels": names_of(&issue["labels"]),
            "body": issue["body"].as_str().unwrap_or(""),
            "comments": list,
        }))
    }
    .await;
    finish(r, &token)
}

async fn label_events(id: &AgentId, kernel: &Arc<Kernel>, cfg: &Cfg, a: &verbs::LabelEvents) -> Value {
    const V: &str = "label_events";
    let repo = match repo_arg(&a.repo, V, cfg) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let number = a.number;
    let api = Api::new(cfg);
    let token = match app::token_for(id, kernel, cfg, &api, V, &repo).await {
        Ok(t) => t,
        Err(e) => return e,
    };
    let r: Result<Value, Value> = async {
        let events = ge(api
            .call(
                V,
                Method::GET,
                &format!("/repos/{repo}/issues/{number}/events"),
                &[("per_page", "100".into())],
                &token,
                None,
            )
            .await)?;
        let list: Vec<Value> = events
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|e| matches!(e["event"].as_str(), Some("labeled" | "unlabeled")))
                    .map(|e| {
                        json!({
                            "event": e["event"],
                            "label": e["label"]["name"],
                            "actor": e["actor"]["login"],
                            "created_at": e["created_at"],
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(json!({"repo": repo, "number": number, "events": list}))
    }
    .await;
    finish(r, &token)
}

async fn comment(id: &AgentId, kernel: &Arc<Kernel>, cfg: &Cfg, a: &verbs::Comment) -> Value {
    const V: &str = "comment";
    let repo = match repo_arg(&a.repo, V, cfg) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let number = a.number;
    let body = a.body.clone();
    let api = Api::new(cfg);
    let token = match app::token_for(id, kernel, cfg, &api, V, &repo).await {
        Ok(t) => t,
        Err(e) => return e,
    };
    let r: Result<Value, Value> = async {
        let made = ge(api
            .call(
                V,
                Method::POST,
                &format!("/repos/{repo}/issues/{number}/comments"),
                &[],
                &token,
                Some(&json!({"body": body})),
            )
            .await)?;
        Ok(json!({
            "repo": repo,
            "number": number,
            "comment_id": made["id"],
            "url": made["html_url"],
        }))
    }
    .await;
    finish(r, &token)
}

async fn label(id: &AgentId, kernel: &Arc<Kernel>, cfg: &Cfg, a: &verbs::Label) -> Value {
    const V: &str = "label";
    let repo = match repo_arg(&a.repo, V, cfg) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let number = a.number;
    let (add, remove) = (a.add.clone(), a.remove.clone());
    if add.is_none() && remove.is_none() {
        return bad(LABEL_TEXT.into(), "add", json!(null));
    }
    let api = Api::new(cfg);
    let token = match app::token_for(id, kernel, cfg, &api, V, &repo).await {
        Ok(t) => t,
        Err(e) => return e,
    };
    let r: Result<Value, Value> = async {
        if let Some(a) = &add {
            ge(api
                .call(
                    V,
                    Method::POST,
                    &format!("/repos/{repo}/issues/{number}/labels"),
                    &[],
                    &token,
                    Some(&json!({"labels": a})),
                )
                .await)?;
        }
        if let Some(rm) = &remove {
            for name in rm {
                match api
                    .call(
                        V,
                        Method::DELETE,
                        &format!("/repos/{repo}/issues/{number}/labels/{}", pct(name)),
                        &[],
                        &token,
                        None,
                    )
                    .await
                {
                    Ok(_) => {}
                    Err((404, _)) => {}
                    Err((_, v)) => return Err(v),
                }
            }
        }
        let now = ge(api
            .call(
                V,
                Method::GET,
                &format!("/repos/{repo}/issues/{number}/labels"),
                &[],
                &token,
                None,
            )
            .await)?;
        Ok(json!({"repo": repo, "number": number, "labels": names_of(&now)}))
    }
    .await;
    finish(r, &token)
}

fn checks_summary(checks: &[Value]) -> &'static str {
    if checks.is_empty() {
        return "none";
    }
    let failed = |c: &Value| {
        matches!(
            c["conclusion"].as_str(),
            Some("failure" | "timed_out" | "cancelled" | "action_required" | "startup_failure")
        )
    };
    if checks.iter().any(failed) {
        return "failure";
    }
    if checks
        .iter()
        .any(|c| c["status"].as_str() != Some("completed"))
    {
        return "pending";
    }
    "success"
}

fn review_summary(reviews: &Value) -> (Value, &'static str) {
    // Each author's LAST review, in first-seen order.
    let mut last: Vec<(String, String)> = Vec::new();
    for r in reviews.as_array().into_iter().flatten() {
        let author = r["user"]["login"].as_str().unwrap_or("").to_string();
        let state = r["state"].as_str().unwrap_or("").to_string();
        match last.iter_mut().find(|(a, _)| *a == author) {
            Some(e) => e.1 = state,
            None => last.push((author, state)),
        }
    }
    let any = |s: &str| last.iter().any(|(_, st)| st == s);
    let summary = if any("CHANGES_REQUESTED") {
        "changes_requested"
    } else if any("APPROVED") {
        "approved"
    } else if !last.is_empty() {
        "commented"
    } else {
        "none"
    };
    let list: Vec<Value> = last
        .iter()
        .map(|(a, s)| json!({"author": a, "state": s}))
        .collect();
    (Value::Array(list), summary)
}

async fn pr_status(id: &AgentId, kernel: &Arc<Kernel>, cfg: &Cfg, a: &verbs::PrStatus) -> Value {
    const V: &str = "pr_status";
    let repo = match repo_arg(&a.repo, V, cfg) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let number = a.number;
    let api = Api::new(cfg);
    let token = match app::token_for(id, kernel, cfg, &api, V, &repo).await {
        Ok(t) => t,
        Err(e) => return e,
    };
    let r: Result<Value, Value> = async {
        let pr = ge(api
            .call(
                V,
                Method::GET,
                &format!("/repos/{repo}/pulls/{number}"),
                &[],
                &token,
                None,
            )
            .await)?;
        let sha = pr["head"]["sha"].as_str().unwrap_or("").to_string();
        let runs = ge(api
            .call(
                V,
                Method::GET,
                &format!("/repos/{repo}/commits/{sha}/check-runs"),
                &[("per_page", "100".into())],
                &token,
                None,
            )
            .await)?;
        let reviews = ge(api
            .call(
                V,
                Method::GET,
                &format!("/repos/{repo}/pulls/{number}/reviews"),
                &[("per_page", "100".into())],
                &token,
                None,
            )
            .await)?;
        let checks: Vec<Value> = runs["check_runs"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|c| {
                        json!({
                            "name": c["name"],
                            "status": c["status"],
                            "conclusion": c["conclusion"],
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let (review_list, review_state) = review_summary(&reviews);
        Ok(json!({
            "repo": repo,
            "number": number,
            "state": pr["state"],
            "draft": pr["draft"],
            "head_sha": sha,
            "checks": checks,
            "checks_summary": checks_summary(&checks),
            "reviews": review_list,
            "review_state": review_state,
        }))
    }
    .await;
    finish(r, &token)
}

/// `merge {repo, number, method?: squash|merge}`: squash-merge a green pull request.
/// Refuses unless it is open, targets the default branch and every check on its head
/// commit passed. A draft is marked ready first.
async fn merge(id: &AgentId, kernel: &Arc<Kernel>, cfg: &Cfg, a: &verbs::Merge) -> Value {
    const V: &str = "merge";
    const READY: &str = "mutation($id: ID!) { markPullRequestReadyForReview(input: {pullRequestId: $id}) { pullRequest { id } } }";
    let repo = match repo_arg(&a.repo, V, cfg) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let number = a.number;
    let method = a.method.as_str().to_string();
    let api = Api::new(cfg);
    let token = match app::token_for(id, kernel, cfg, &api, V, &repo).await {
        Ok(t) => t,
        Err(e) => return e,
    };
    let r: Result<Value, Value> = async {
        let info = ge(api
            .call(V, Method::GET, &format!("/repos/{repo}"), &[], &token, None)
            .await)?;
        let default = info["default_branch"].as_str().unwrap_or("").to_string();
        let pr = ge(api
            .call(V, Method::GET, &format!("/repos/{repo}/pulls/{number}"), &[], &token, None)
            .await)?;
        let state = pr["state"].as_str().unwrap_or("").to_string();
        if state != "open" {
            return Err(bad(
                format!("team_github merge: #{number} is {state}, not open"),
                "state",
                json!(state),
            ));
        }
        let base = pr["base"]["ref"].as_str().unwrap_or("").to_string();
        if base != default {
            return Err(bad(
                format!("team_github merge: #{number} targets `{base}`, not the default branch `{default}`"),
                "base",
                json!(base),
            ));
        }
        let sha = pr["head"]["sha"].as_str().unwrap_or("").to_string();
        let runs = ge(api
            .call(
                V,
                Method::GET,
                &format!("/repos/{repo}/commits/{sha}/check-runs"),
                &[("per_page", "100".into())],
                &token,
                None,
            )
            .await)?;
        let checks: Vec<Value> = runs["check_runs"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|c| json!({"name": c["name"], "status": c["status"], "conclusion": c["conclusion"]}))
                    .collect()
            })
            .unwrap_or_default();
        let summary = checks_summary(&checks);
        if summary != "success" {
            let why = if checks.is_empty() {
                "there are no checks on the head commit".to_string()
            } else {
                let open: Vec<String> = checks
                    .iter()
                    .filter(|c| {
                        c["status"].as_str() != Some("completed")
                            || matches!(
                                c["conclusion"].as_str(),
                                Some("failure" | "timed_out" | "cancelled" | "action_required" | "startup_failure")
                            )
                    })
                    .map(|c| {
                        format!(
                            "{} ({})",
                            c["name"].as_str().unwrap_or("?"),
                            c["conclusion"].as_str().or(c["status"].as_str()).unwrap_or("?")
                        )
                    })
                    .collect();
                format!("not passed: {}", open.join(", "))
            };
            return Err(bad(
                format!("team_github merge: #{number} is not green ({summary}): {why}"),
                "number",
                json!(number),
            ));
        }
        if pr["draft"].as_bool() == Some(true) {
            let reply = ge(api
                .call(
                    V,
                    Method::POST,
                    "/graphql",
                    &[],
                    &token,
                    Some(&json!({"query": READY, "variables": {"id": pr["node_id"]}})),
                )
                .await)?;
            if reply["errors"].is_array() {
                return Err(bad(
                    format!("team_github merge: #{number} could not be marked ready: {}", reply["errors"]),
                    "number",
                    json!(number),
                ));
            }
        }
        let made = ge(api
            .call(
                V,
                Method::PUT,
                &format!("/repos/{repo}/pulls/{number}/merge"),
                &[],
                &token,
                Some(&json!({"merge_method": method, "sha": sha})),
            )
            .await)?;
        if made["merged"].as_bool() != Some(true) {
            return Err(bad(
                format!(
                    "team_github merge: GitHub did not merge #{number}: {}",
                    made["message"].as_str().unwrap_or("no message")
                ),
                "number",
                json!(number),
            ));
        }
        Ok(json!({"merged": true, "sha": made["sha"], "number": number}))
    }
    .await;
    let mut out = finish(r, &token);
    if out.get("error").is_some() && out.get("reason").is_none() {
        out["reason"] = json!("github_error");
    }
    out
}

/// The most commits and files `staleness` lists for one repo.
const STALE_COMMITS: usize = 50;
const STALE_FILES: usize = 100;

/// What changed on a branch since a recorded commit, from GitHub's compare answer.
/// Pure: `cmp` is the compare reply (null when none was asked), `commit` the
/// recorded commit, `head` the branch head. It gives facts and decides nothing.
pub(crate) fn summarize_compare(cmp: &Value, commit: Option<&str>, head: &str) -> Value {
    let Some(commit) = commit else {
        return json!({"state": "unknown", "ahead_by": null, "behind_by": null,
                      "commits": [], "files": [], "truncated": false});
    };
    let state = if commit == head {
        "same"
    } else {
        match cmp["status"].as_str() {
            Some("identical") => "same",
            Some("ahead") => "ahead",
            // diverged, or the recorded commit is past the branch: no longer an ancestor
            _ => "diverged",
        }
    };
    let all_commits = cmp["commits"].as_array().cloned().unwrap_or_default();
    let all_files = cmp["files"].as_array().cloned().unwrap_or_default();
    let ahead_by = cmp["ahead_by"].as_u64().unwrap_or(0);
    let listed: Vec<Value> = all_commits
        .iter()
        .take(STALE_COMMITS)
        .map(|c| {
            let sha: String = c["sha"].as_str().unwrap_or("").chars().take(12).collect();
            let subject = c["commit"]["message"]
                .as_str()
                .and_then(|m| m.lines().next())
                .unwrap_or("");
            json!({"sha": sha, "subject": subject})
        })
        .collect();
    let paths: Vec<Value> = all_files
        .iter()
        .take(STALE_FILES)
        .filter_map(|f| f["filename"].as_str().map(|n| json!(n)))
        .collect();
    let truncated = all_commits.len() > STALE_COMMITS
        || all_files.len() > STALE_FILES
        || ahead_by > all_commits.len() as u64;
    json!({
        "state": state,
        "ahead_by": ahead_by,
        "behind_by": cmp["behind_by"].as_u64().unwrap_or(0),
        "commits": listed,
        "files": paths,
        "truncated": truncated,
    })
}

/// The issue facts: its `updated_at`, and whether it differs from the recorded
/// `source_version` (null when none was recorded).
pub(crate) fn issue_facts(issue: &Value, source_version: Option<&str>) -> Value {
    let updated = issue["updated_at"].as_str();
    let changed = match (source_version, updated) {
        (Some(sv), Some(u)) => json!(sv != u),
        (Some(_), None) => json!(true),
        (None, _) => Value::Null,
    };
    json!({"number": issue["number"], "updated_at": updated, "changed": changed})
}

/// `staleness {versions?: [{repo, branch, commit?}], issue?: {repo, number,
/// source_version?}}`: facts for reusing a mark. Per repo, the branch head and (given
/// a recorded commit) the commits and files since it; for the issue, its `updated_at`
/// and whether it moved since `source_version`. Every repo is checked against the
/// allowlist before any request; a token is minted per repo.
async fn staleness(id: &AgentId, kernel: &Arc<Kernel>, cfg: &Cfg, a: &verbs::Staleness) -> Value {
    const V: &str = "staleness";
    let versions = a.versions.as_deref();
    let issue = a.issue.as_ref();
    if versions.is_none_or(|a| a.is_empty()) && issue.is_none() {
        return missing(V, "versions");
    }
    // validate everything before the first request
    let mut asked: Vec<(String, String, Option<String>)> = Vec::new();
    for e in versions.into_iter().flatten() {
        let repo = match repo_arg(&e.repo, V, cfg) {
            Ok(r) => r,
            Err(e) => return e,
        };
        asked.push((repo, e.branch.clone(), e.commit.clone()));
    }
    let mut asked_issue: Option<(String, u64, Option<String>)> = None;
    if let Some(i) = issue {
        let repo = match repo_arg(&i.repo, V, cfg) {
            Ok(r) => r,
            Err(e) => return e,
        };
        asked_issue = Some((repo, i.number, i.source_version.clone()));
    }
    let api = Api::new(cfg);
    let mut tokens: Vec<(String, String)> = Vec::new();
    let r: Result<Value, Value> = async {
        let mut out = serde_json::Map::new();
        if versions.is_some_and(|a| !a.is_empty()) {
            let mut repos = Vec::new();
            for (repo, branch, commit) in &asked {
                let token = stale_token(id, kernel, cfg, &api, &mut tokens, repo).await?;
                let seg: String = branch.split('/').map(pct).collect::<Vec<_>>().join("/");
                let b = ge(api
                    .call(
                        V,
                        Method::GET,
                        &format!("/repos/{repo}/branches/{seg}"),
                        &[],
                        &token,
                        None,
                    )
                    .await)?;
                let head = b["commit"]["sha"].as_str().unwrap_or("").to_string();
                let cmp = match commit {
                    Some(c) if *c != head => ge(api
                        .call(
                            V,
                            Method::GET,
                            &format!("/repos/{repo}/compare/{}...{seg}", pct(c)),
                            &[("per_page", "100".into())],
                            &token,
                            None,
                        )
                        .await)?,
                    _ => Value::Null,
                };
                let mut entry = summarize_compare(&cmp, commit.as_deref(), &head);
                entry["repo"] = json!(repo);
                entry["branch"] = json!(branch);
                entry["head"] = json!(head);
                repos.push(entry);
            }
            out.insert("repos".into(), Value::Array(repos));
        }
        if let Some((repo, number, sv)) = &asked_issue {
            let token = stale_token(id, kernel, cfg, &api, &mut tokens, repo).await?;
            let v = ge(api
                .call(
                    V,
                    Method::GET,
                    &format!("/repos/{repo}/issues/{number}"),
                    &[],
                    &token,
                    None,
                )
                .await)?;
            let mut facts = issue_facts(&v, sv.as_deref());
            facts["repo"] = json!(repo);
            facts["number"] = json!(number);
            out.insert("issue".into(), facts);
        }
        Ok(Value::Object(out))
    }
    .await;
    let mut v = match r {
        Ok(v) | Err(v) => v,
    };
    for (_, t) in &tokens {
        scrub_all(&mut v, &[t.as_str()]);
    }
    v
}

/// The token for `repo`, minted once per call.
async fn stale_token(
    id: &AgentId,
    kernel: &Arc<Kernel>,
    cfg: &Cfg,
    api: &Api,
    tokens: &mut Vec<(String, String)>,
    repo: &str,
) -> Result<String, Value> {
    if let Some((_, t)) = tokens.iter().find(|(r, _)| r == repo) {
        return Ok(t.clone());
    }
    let t = app::token_for(id, kernel, cfg, api, "staleness", repo).await?;
    tokens.push((repo.to_string(), t.clone()));
    Ok(t)
}

/// True when line `i` sits under a `## SPEC` heading (until the next `## ` heading).
fn in_spec_section(lines: &[&str], i: usize) -> bool {
    let mut spec = false;
    for l in &lines[..=i] {
        if let Some(h) = l.strip_prefix("## ") {
            spec = h.trim() == "SPEC";
        }
    }
    spec
}

async fn issue_edit(id: &AgentId, kernel: &Arc<Kernel>, cfg: &Cfg, a: &verbs::IssueEdit) -> Value {
    const V: &str = "issue_edit";
    let repo = match repo_arg(&a.repo, V, cfg) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let number = a.number;
    let old = a.old.clone();
    let new = a.new.clone();
    let api = Api::new(cfg);
    let token = match app::token_for(id, kernel, cfg, &api, V, &repo).await {
        Ok(t) => t,
        Err(e) => return e,
    };
    let r: Result<Value, Value> = async {
        let issue = ge(api
            .call(
                V,
                Method::GET,
                &format!("/repos/{repo}/issues/{number}"),
                &[],
                &token,
                None,
            )
            .await)?;
        let body = issue["body"].as_str().unwrap_or("").to_string();
        let lines: Vec<&str> = body.split('\n').collect();
        let hits: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.trim_end() == old.trim_end())
            .map(|(i, _)| i)
            .collect();
        if hits.is_empty() {
            return Err(bad(
                format!("team_github issue_edit: no line of #{number} matches `{old}`"),
                "old",
                json!(old),
            ));
        }
        if hits.len() > 1 {
            return Err(bad(
                format!(
                    "team_github issue_edit: {} lines of #{number} match `{old}` — give a unique line",
                    hits.len()
                ),
                "old",
                json!(old),
            ));
        }
        let i = hits[0];
        if in_spec_section(&lines, i) {
            return Err(bad(
                format!(
                    "team_github issue_edit: line {} of #{number} is inside a `## SPEC` section — the keeper does not edit specs",
                    i + 1
                ),
                "old",
                json!(old),
            ));
        }
        let mut out = lines;
        out[i] = new.as_str();
        let text = out.join("\n");
        ge(api
            .call(
                V,
                Method::PATCH,
                &format!("/repos/{repo}/issues/{number}"),
                &[],
                &token,
                Some(&json!({"body": text})),
            )
            .await)?;
        Ok(json!({"repo": repo, "number": number, "line": i + 1}))
    }
    .await;
    finish(r, &token)
}

async fn project_field_set(id: &AgentId, kernel: &Arc<Kernel>, cfg: &Cfg, a: &verbs::ProjectFieldSet) -> Value {
    const V: &str = "project_field_set";
    let repo = match repo_arg(&a.repo, V, cfg) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let project = a.project.clone();
    let item = a.item.clone();
    let field = a.field.clone();
    let value = a.value.clone();
    let api = Api::new(cfg);
    let token = match app::token_for(id, kernel, cfg, &api, V, &repo).await {
        Ok(t) => t,
        Err(e) => return e,
    };
    const MUTATION: &str = "mutation($p: ID!, $i: ID!, $f: ID!, $v: String!) { updateProjectV2ItemFieldValue(input: {projectId: $p, itemId: $i, fieldId: $f, value: {text: $v}}) { projectV2Item { id } } }";
    let r: Result<Value, Value> = async {
        let reply = ge(api
            .call(
                V,
                Method::POST,
                "/graphql",
                &[],
                &token,
                Some(&json!({
                    "query": MUTATION,
                    "variables": {"p": project, "i": item, "f": field, "v": value},
                })),
            )
            .await)?;
        // GraphQL reports failures inside a 200 reply.
        if reply["errors"].is_array() {
            return Err(bad(
                format!("team_github project_field_set: {}", reply["errors"]),
                "field",
                json!(field),
            ));
        }
        Ok(json!({"repo": repo, "item": item, "field": field}))
    }
    .await;
    finish(r, &token)
}

/// Open an issue on a repo as the app. With `project`, also add it to that board.
async fn issue_create(id: &AgentId, kernel: &Arc<Kernel>, cfg: &Cfg, a: &verbs::IssueCreate) -> Value {
    const V: &str = "issue_create";
    let repo = match repo_arg(&a.repo, V, cfg) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let title = a.title.clone();
    let body = a.body.clone();
    let project = a.project.clone();
    let api = Api::new(cfg);
    let token = match app::token_for(id, kernel, cfg, &api, V, &repo).await {
        Ok(t) => t,
        Err(e) => return e,
    };
    const ADD: &str = "mutation($project: ID!, $content: ID!) { addProjectV2ItemById(input: {projectId: $project, contentId: $content}) { item { id } } }";
    let r: Result<Value, Value> = async {
        let issue = ge(api
            .call(
                V,
                Method::POST,
                &format!("/repos/{repo}/issues"),
                &[],
                &token,
                Some(&json!({"title": title, "body": body})),
            )
            .await)?;
        let number = issue["number"].clone();
        let mut added = false;
        if let Some(proj) = &project {
            let reply = ge(api
                .call(
                    V,
                    Method::POST,
                    "/graphql",
                    &[],
                    &token,
                    Some(&json!({
                        "query": ADD,
                        "variables": {"project": proj, "content": issue["node_id"]},
                    })),
                )
                .await)?;
            // GraphQL reports failures inside a 200 reply.
            if reply["errors"].is_array() {
                return Err(bad(
                    format!(
                        "team_github issue_create: #{number} was created but not added to the board: {}",
                        reply["errors"]
                    ),
                    "project",
                    json!(proj),
                ));
            }
            added = true;
        }
        Ok(json!({
            "repo": repo,
            "number": number,
            "url": issue["html_url"],
            "added_to_board": added,
        }))
    }
    .await;
    finish(r, &token)
}

/// Open or close an issue. Closing an issue labelled `epic` is the maintainer's alone.
async fn issue_state(id: &AgentId, kernel: &Arc<Kernel>, cfg: &Cfg, a: &verbs::IssueState) -> Value {
    const V: &str = "issue_state";
    let repo = match repo_arg(&a.repo, V, cfg) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let number = a.number;
    let state = a.state.as_str().to_string();
    let sender = a.sender.as_deref().unwrap_or("");
    let api = Api::new(cfg);
    let token = match app::token_for(id, kernel, cfg, &api, V, &repo).await {
        Ok(t) => t,
        Err(e) => return e,
    };
    let r: Result<Value, Value> = async {
        let issue = ge(api
            .call(
                V,
                Method::GET,
                &format!("/repos/{repo}/issues/{number}"),
                &[],
                &token,
                None,
            )
            .await)?;
        if issue.get("pull_request").is_some() {
            return Err(bad(
                format!("team_github issue_state: #{number} is a pull request, not an issue"),
                "number",
                json!(number),
            ));
        }
        let epic = names_of(&issue["labels"])
            .iter()
            .any(|l| l.eq_ignore_ascii_case("epic"));
        if state == "closed" && epic && sender != "maintainer" {
            return Err(bad(
                format!("team_github issue_state: #{number} is labelled `epic`; only the maintainer closes it"),
                "number",
                json!(number),
            ));
        }
        ge(api
            .call(
                V,
                Method::PATCH,
                &format!("/repos/{repo}/issues/{number}"),
                &[],
                &token,
                Some(&json!({"state": state})),
            )
            .await)?;
        Ok(json!({"repo": repo, "number": number, "state": state}))
    }
    .await;
    finish(r, &token)
}

#[cfg(test)]
mod keeper_tests {
    use super::*;

    #[test]
    fn spec_section_is_detected_until_next_heading() {
        let lines = ["# T", "## SPEC", "- [ ] a", "## Notes", "- [ ] b"];
        assert!(in_spec_section(&lines, 2));
        assert!(!in_spec_section(&lines, 4));
        assert!(!in_spec_section(&lines, 0));
    }

    #[test]
    fn issue_edit_refuses_spec_section() {
        let lines = ["## SPEC", "- [ ] a"];
        assert!(in_spec_section(&lines, 1));
    }
}
