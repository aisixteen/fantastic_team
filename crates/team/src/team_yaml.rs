//! `team.yaml`: a team's setup as one preset yaml in the team folder (#292).
//!
//! The file declares the team's own tree and nothing else (no tabs, no kernel list): the
//! provider (`anthropic_backend`: endpoint, `caching`, models, the default model, the
//! credential ref), the credential store's refs (the NAMES of environment variables, never
//! a value), the connectors per repo (a git and a github agent, the bot's author identity,
//! the repo allowlist, the directive), the worktree root, the roles' limits, and the lead's
//! provider and model.
//!
//! [`parse`] checks it (a missing field fails naming it), [`overlay`] folds it into the
//! team's blueprint ([`crate::hydrate::blueprint_text`] does it, so the records are SEEDED
//! with everything else and nothing is created at a boot), and [`apply`] is the first start
//! of a fresh folder: seed, then keep the default model in the provider's memory store (a
//! `set_default_model` on a kernel that is shut down again). [`run_check`] is the dry
//! check (`fantastic_team check`), made before the first model call: it applies a fresh
//! folder, then checks the records and asks each github connector `whoami`. No model is
//! called.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use fantastic_kernel::bootstrap::{self, DEFAULT_ROOT_ID};
use fantastic_kernel::{Item, Status};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::new_team::{
    BOARD, CREDENTIALS, GITHUB, LEAD, MEMORY, PARENT, ROLES, WORKER_EXEC, WORKER_FILES,
};

/// The preset's file name, in the team folder.
pub const FILE: &str = "team.yaml";
/// The provider's id when the preset names none.
pub const PROVIDER_ID: &str = "anthropic";
/// The git connector of the first (and the default) repo; the role cards name it.
pub const GIT: &str = "git";
/// The only `caching` a preset may declare: the provider's own prompt cache.
pub const CACHING: &str = "provider_cache";
/// A connector's directive when the preset names none.
const DIRECTIVE: &str = "Push a work branch and open a draft pull request. Never the default branch, never a force, never a merge.";

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    credentials: Option<BTreeMap<String, RawRef>>,
    provider: Option<RawProvider>,
    connectors: Option<Vec<RawConnector>>,
    worktrees: Option<String>,
    roles: Option<RawRoles>,
    lead: Option<RawLead>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRef {
    host: Option<String>,
    env: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProvider {
    id: Option<String>,
    endpoint: Option<String>,
    caching: Option<String>,
    models: Option<Vec<String>>,
    default_model: Option<String>,
    context_window: Option<u64>,
    credentials: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConnector {
    name: Option<String>,
    repo: Option<String>,
    clone: Option<String>,
    app: Option<String>,
    author: Option<RawAuthor>,
    remote: Option<String>,
    default_branch: Option<String>,
    api_url: Option<String>,
    directive: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAuthor {
    name: Option<String>,
    email: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRoles {
    limits: Option<RawLimits>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLimits {
    total: Option<u64>,
    per_role: Option<BTreeMap<String, u64>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLead {
    provider: Option<String>,
    model: Option<String>,
}

/// One credential ref: the host it is bound to and, per field, the NAME of the environment
/// variable the credential store reads it from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredRef {
    /// The host the ref is bound to.
    pub host: String,
    /// Field name to environment variable name.
    pub env: BTreeMap<String, String>,
}

/// The provider agent's declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    /// Its agent id.
    pub id: String,
    /// The API endpoint.
    pub endpoint: String,
    /// The caching mode ([`CACHING`]).
    pub caching: String,
    /// The models it serves.
    pub models: Vec<String>,
    /// The default model, kept in its memory store at the first start.
    pub default_model: Option<String>,
    /// The context window, when declared.
    pub context_window: Option<u64>,
    /// The credential ref its key is read through.
    pub credentials: String,
}

/// One repo's git and github agents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connector {
    /// The git connector's id.
    pub git_id: String,
    /// The github connector's id.
    pub github_id: String,
    /// `owner/name`, the connector's allowlist.
    pub repo: String,
    /// The local clone (the git root), inside the team folder.
    pub clone: PathBuf,
    /// The credential ref of the GitHub App.
    pub app: String,
    /// The bot's commit author name.
    pub author_name: String,
    /// The bot's commit author email.
    pub author_email: String,
    /// The remote name or URL.
    pub remote: String,
    /// The remote's default branch.
    pub default_branch: String,
    /// The GitHub API root, when not github.com's.
    pub api_url: Option<String>,
    /// What the connector may be used for.
    pub directive: String,
}

/// A checked `team.yaml`: every required field is present and every path is inside the
/// team folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preset {
    /// The credential store's refs, by name.
    pub refs: BTreeMap<String, CredRef>,
    /// The provider.
    pub provider: Provider,
    /// The connectors, the first being the `git` / `github` the role cards name.
    pub connectors: Vec<Connector>,
    /// The worktree root, inside the team folder.
    pub worktrees: PathBuf,
    /// The roles' total limit.
    pub total: u64,
    /// The roles' per-role limits.
    pub per_role: BTreeMap<String, u64>,
    /// The lead's model.
    pub lead_model: String,
}

/// What a check of the preset found missing or wrong.
#[derive(Default)]
struct Problems {
    missing: Vec<String>,
    bad: Vec<String>,
}

impl Problems {
    fn need<T>(&mut self, v: Option<T>, path: &str) -> Option<T> {
        if v.is_none() {
            self.missing.push(path.to_string());
        }
        v
    }

    fn text(&mut self, v: Option<String>, path: &str) -> Option<String> {
        match v.map(|s| s.trim().to_string()) {
            Some(s) if !s.is_empty() => Some(s),
            _ => {
                self.missing.push(path.to_string());
                None
            }
        }
    }

    fn bad(&mut self, what: String) {
        self.bad.push(what);
    }

    fn done(self) -> Result<(), String> {
        let mut parts = Vec::new();
        if !self.missing.is_empty() {
            parts.push(format!("missing {}", self.missing.join(", ")));
        }
        parts.extend(self.bad);
        if parts.is_empty() {
            Ok(())
        } else {
            Err(format!("{FILE}: {}", parts.join("; ")))
        }
    }
}

/// `p` made absolute inside `dir`, `.` and `..` folded; refused when it is `dir` itself or
/// outside it.
fn under(dir: &Path, p: &str) -> Result<PathBuf, String> {
    let raw = Path::new(p);
    let joined = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        dir.join(raw)
    };
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    if out.starts_with(dir) && out != dir {
        Ok(out)
    } else {
        Err(format!("`{p}` is not a folder inside the team folder"))
    }
}

/// A name an environment variable can have. A key's own text (`sk-…`, a PEM) is not one.
fn env_name(s: &str) -> bool {
    let mut cs = s.chars();
    cs.next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && cs.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn repo_name(s: &str) -> bool {
    let part = |p: &str| {
        !p.is_empty()
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    };
    s.split_once('/')
        .is_some_and(|(o, n)| part(o) && part(n) && !n.contains('/'))
}

/// The suffix of a connector's agent ids: its `name`, else the repo's name, lower case and
/// `_` for anything that is not a letter or a digit.
fn slug(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Check `text` (the team folder is `dir`). Every missing field is named, by its path in
/// the file, in one error.
pub fn parse(text: &str, dir: &Path) -> Result<Preset, String> {
    let raw: Raw = serde_yaml::from_str(text).map_err(|e| format!("{FILE}: {e}"))?;
    let mut p = Problems::default();

    let mut refs = BTreeMap::new();
    for (name, r) in p.need(raw.credentials, "credentials").unwrap_or_default() {
        let host = p.text(r.host, &format!("credentials.{name}.host"));
        let env = p.need(r.env, &format!("credentials.{name}.env"));
        if env.as_ref().is_some_and(BTreeMap::is_empty) {
            p.missing.push(format!("credentials.{name}.env"));
        }
        let env = env.unwrap_or_default();
        for (field, var) in &env {
            if !env_name(var) {
                p.bad(format!(
                    "credentials.{name}.env.{field} must name an environment variable, never a value"
                ));
            }
        }
        if let Some(host) = host {
            refs.insert(name, CredRef { host, env });
        }
    }
    if refs.is_empty() && !p.missing.iter().any(|m| m == "credentials") {
        p.missing.push("credentials".into());
    }

    let rp = p.need(raw.provider, "provider").unwrap_or_default();
    let provider = {
        let id = rp
            .id
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| PROVIDER_ID.into());
        let endpoint = p.text(rp.endpoint, "provider.endpoint");
        let caching = p.text(rp.caching, "provider.caching");
        if let Some(c) = caching.as_deref().filter(|c| *c != CACHING) {
            p.bad(format!("provider.caching must be {CACHING} (got {c})"));
        }
        let models = rp.models.unwrap_or_default();
        if models.is_empty() {
            p.missing.push("provider.models".into());
        }
        let credentials = p.text(rp.credentials, "provider.credentials");
        if let Some(c) = credentials.as_deref().filter(|c| !refs.contains_key(*c)) {
            p.bad(format!(
                "provider.credentials names `{c}`, which credentials does not declare"
            ));
        }
        if let Some(d) = rp
            .default_model
            .as_deref()
            .filter(|d| !models.iter().any(|m| m == d))
        {
            p.bad(format!(
                "provider.default_model `{d}` is not one of provider.models"
            ));
        }
        Provider {
            id,
            endpoint: endpoint.unwrap_or_default(),
            caching: caching.unwrap_or_default(),
            models,
            default_model: rp.default_model,
            context_window: rp.context_window,
            credentials: credentials.unwrap_or_default(),
        }
    };

    let mut connectors: Vec<Connector> = Vec::new();
    let raw_connectors = raw.connectors.unwrap_or_default();
    if raw_connectors.is_empty() {
        p.missing.push("connectors".into());
    }
    for (i, c) in raw_connectors.into_iter().enumerate() {
        let at = |f: &str| format!("connectors[{i}].{f}");
        let repo = p.text(c.repo, &at("repo"));
        if let Some(r) = repo.as_deref().filter(|r| !repo_name(r)) {
            p.bad(format!("{} must be owner/name (got `{r}`)", at("repo")));
        }
        let clone = p.text(c.clone, &at("clone")).and_then(|s| {
            under(dir, &s)
                .map_err(|e| p.bad(format!("{}: {e}", at("clone"))))
                .ok()
        });
        let app = p.text(c.app, &at("app"));
        if let Some(a) = app.as_deref().filter(|a| !refs.contains_key(*a)) {
            p.bad(format!(
                "{} names `{a}`, which credentials does not declare",
                at("app")
            ));
        }
        let author = p.need(c.author, &at("author")).unwrap_or_default();
        let author_name = p.text(author.name, &at("author.name"));
        let author_email = p.text(author.email, &at("author.email"));
        let (Some(repo), Some(clone), Some(app), Some(author_name), Some(author_email)) =
            (repo, clone, app, author_name, author_email)
        else {
            continue;
        };
        let suffix = slug(
            c.name
                .as_deref()
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| repo.rsplit('/').next().unwrap_or("")),
        );
        let (git_id, github_id) = if connectors.is_empty() {
            (GIT.to_string(), GITHUB.to_string())
        } else {
            (format!("{GIT}_{suffix}"), format!("{GITHUB}_{suffix}"))
        };
        if connectors
            .iter()
            .any(|o| o.git_id == git_id || o.repo == repo)
        {
            p.bad(format!(
                "{} repeats a repo or a name (ids `{git_id}`)",
                at("repo")
            ));
            continue;
        }
        connectors.push(Connector {
            git_id,
            github_id,
            repo,
            clone,
            app,
            author_name,
            author_email,
            remote: c
                .remote
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "origin".into()),
            default_branch: c
                .default_branch
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "main".into()),
            api_url: c.api_url.filter(|s| !s.is_empty()),
            directive: c
                .directive
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| DIRECTIVE.into()),
        });
    }

    let worktrees = p.text(raw.worktrees, "worktrees").and_then(|s| {
        under(dir, &s)
            .map_err(|e| p.bad(format!("worktrees: {e}")))
            .ok()
    });

    let limits = p
        .need(raw.roles.and_then(|r| r.limits), "roles.limits")
        .unwrap_or_default();
    let total = p.need(limits.total, "roles.limits.total");

    let lead = p.need(raw.lead, "lead").unwrap_or_default();
    if let Some(l) = lead.provider.as_deref().filter(|l| *l != provider.id) {
        p.bad(format!(
            "lead.provider `{l}` is not the declared provider `{}`",
            provider.id
        ));
    }
    let lead_model = p.text(lead.model, "lead.model");
    if let Some(m) = lead_model
        .as_deref()
        .filter(|m| !provider.models.iter().any(|x| x == *m))
    {
        p.bad(format!("lead.model `{m}` is not one of provider.models"));
    }

    p.done()?;
    Ok(Preset {
        refs,
        provider,
        connectors,
        worktrees: worktrees.unwrap_or_default(),
        total: total.unwrap_or_default(),
        per_role: limits.per_role.unwrap_or_default(),
        lead_model: lead_model.unwrap_or_default(),
    })
}

/// The preset of `dir`: `None` when it holds no `team.yaml`.
pub fn load(dir: &Path) -> Result<Option<Preset>, String> {
    match std::fs::read_to_string(dir.join(FILE)) {
        Ok(text) => parse(&text, dir).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{FILE}: {e}")),
    }
}

fn kids(v: &mut Value) -> &mut Vec<Value> {
    let o = v.as_object_mut().expect("an agent entry is a map");
    o.entry("agents")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .expect("`agents` is a list")
}

fn entry<'a>(list: &'a mut [Value], id: &str) -> Result<&'a mut Value, String> {
    list.iter_mut()
        .find(|e| e["id"] == id)
        .ok_or_else(|| format!("blueprint: no `{id}` entry for {FILE} to extend"))
}

/// Add `verbs` for each of `who` to the grant map `field` of `rec` (a union is not needed:
/// the preset's agents are new, and the blueprint's own grants name other agents).
fn grant(rec: &mut Value, field: &str, who: &str, verbs: Value) {
    let o = rec.as_object_mut().expect("an agent entry is a map");
    o.entry(field)
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("a grant is a map")
        .insert(who.to_string(), verbs);
}

/// Fold the preset into the team's blueprint `doc` (the parsed `blueprint.yaml`, its
/// `@work` already filled): the credential store, the provider and the connectors are new
/// records; the lead, the first github connector, the roles registry, the board and the
/// worker's tools get the preset's values. Every edge is written on both ends.
pub fn overlay(doc: &mut Value, p: &Preset) -> Result<(), String> {
    let top = doc["agents"]
        .as_array_mut()
        .ok_or("blueprint: top level must be `agents: [...]`")?;
    let user = entry(top, PARENT)?;
    let github_ids: Vec<&str> = p.connectors.iter().map(|c| c.github_id.as_str()).collect();

    // the credential store: refs by name, the values come from the environment
    let refs: Map<String, Value> = p
        .refs
        .iter()
        .map(|(n, r)| (n.clone(), json!({"host": r.host, "env": r.env})))
        .collect();
    let mut cred = json!({
        "id": CREDENTIALS, "handler_module": "credential_store.tools", "backend": "env",
        "refs": refs,
        "callable_by": {"$self": "*", "$parent": "*", &p.provider.id: ["get"]},
    });
    for g in &github_ids {
        grant(&mut cred, "callable_by", g, json!(["get"]));
    }
    kids(user).push(cred);

    let lead = entry(kids(user), LEAD)?;
    lead["provider_id"] = json!(p.provider.id);
    lead["model"] = json!(p.lead_model);
    grant(
        lead,
        "can_call",
        &p.provider.id,
        json!(["complete", "watch", "models"]),
    );
    for c in &p.connectors {
        grant(lead, "can_call", &c.git_id, json!("*"));
        grant(lead, "can_call", &c.github_id, json!("*"));
    }

    let mut provider = json!({
        "id": p.provider.id, "handler_module": "anthropic_backend.tools",
        "endpoint": p.provider.endpoint, "caching": p.provider.caching, "models": p.provider.models,
        "credentials": {"store": CREDENTIALS, "ref": p.provider.credentials},
        "memory_store_id": MEMORY,
        "can_call": {"$self": "*", CREDENTIALS: ["get"], MEMORY: "*"},
        "callable_by": {"$self": "*", "$parent": ["complete", "watch", "models", "set_default_model"]},
    });
    if let Some(w) = p.provider.context_window {
        provider["context_window"] = json!(w);
    }

    let worktrees = p.worktrees.to_string_lossy().into_owned();
    let lead_kids = kids(lead);
    let template = entry(lead_kids, GITHUB)?.clone();
    let mut added = vec![provider];
    for (i, c) in p.connectors.iter().enumerate() {
        added.push(json!({
            "id": c.git_id, "handler_module": "team_git.tools",
            "repo_dir": c.clone, "workspace_root": p.worktrees.join(&c.git_id),
            "remote": c.remote, "default_branch": c.default_branch,
            "author_name": c.author_name, "author_email": c.author_email,
            "callable_by": {"$self": "*", "$parent": "*", &c.github_id: "*"},
        }));
        let mut gh = template.clone();
        gh["id"] = json!(c.github_id);
        gh["app"] = json!({"credentials": {"store": CREDENTIALS, "ref": c.app}});
        gh["repos"] = json!([c.repo]);
        gh["git"] = json!(c.git_id);
        gh["directive"] = json!(c.directive);
        if let Some(u) = &c.api_url {
            gh["api_url"] = json!(u);
        }
        grant(&mut gh, "can_call", &c.git_id, json!("*"));
        if i == 0 {
            *entry(lead_kids, GITHUB)? = gh;
        } else {
            added.push(gh);
        }
    }
    let all_repos: Vec<&str> = p.connectors.iter().map(|c| c.repo.as_str()).collect();
    entry(lead_kids, BOARD)?["repos"] = json!(all_repos);
    let limits = entry(lead_kids, ROLES)?;
    limits["limits"] = json!({"total": p.total, "per_role": p.per_role});
    entry(lead_kids, WORKER_FILES)?["root"] = json!(worktrees);
    entry(lead_kids, WORKER_EXEC)?["cwd"] = json!(worktrees);
    lead_kids.extend(added);

    // the provider keeps its default model in the shared memory store
    top.push(json!({"id": MEMORY, "callable_by": {&p.provider.id: "*"}}));
    Ok(())
}

/// Whether `dir` has been started in already (fantastic's own test for a fresh folder).
fn started(dir: &Path) -> bool {
    dir.join(".fantastic").exists()
}

/// THE FIRST START of a fresh team folder that holds a `team.yaml`: seed the records
/// ([`crate::hydrate::seed`] folds the preset in), then keep the preset's default model in
/// the provider's memory store. A folder that was started before, or one with no preset,
/// is left alone (`Ok(false)`): the records on disk are the team from then on.
pub fn apply(dir: &Path) -> Result<bool, String> {
    if started(dir) || !dir.join(FILE).exists() {
        return Ok(false);
    }
    let preset = load(dir)?.ok_or("no team.yaml")?;
    if !crate::hydrate::seed(dir)? {
        return Ok(false);
    }
    if let Some(model) = &preset.provider.default_model {
        let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
        rt.block_on(set_default_model(dir, &preset.provider.id, model))?;
    }
    Ok(true)
}

async fn set_default_model(dir: &Path, provider: &str, model: &str) -> Result<(), String> {
    let kernel = crate::start_all_offline(dir)
        .await
        .map_err(|e| e.to_string())?;
    let out = kernel
        .send(vec![Item::new(
            provider,
            json!({"type": "set_default_model", "model": model}),
        )])
        .await;
    let res = match out.first() {
        Some(o) if matches!(o.status, Status::Completed) && o.result.get("error").is_none() => {
            Ok(())
        }
        Some(o) => Err(format!("set_default_model on `{provider}`: {}", o.result)),
        None => Err(format!("set_default_model on `{provider}`: no reply")),
    };
    kernel.flush_records().await;
    kernel.shutdown().await;
    fantastic_sqlite_store::close_all(&kernel);
    let released = bootstrap::shutdown(dir).map_err(|e| e.to_string());
    res.and(released)
}

/// One line of the dry check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    /// What was checked.
    pub what: String,
    /// `None` when it holds, else why not.
    pub failed: Option<String>,
}

fn ok(what: impl Into<String>) -> Check {
    Check {
        what: what.into(),
        failed: None,
    }
}

fn fail(what: impl Into<String>, why: impl Into<String>) -> Check {
    Check {
        what: what.into(),
        failed: Some(why.into()),
    }
}

fn is_under(dir: &Path, v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        let p = Path::new(s);
        p.is_absolute()
            && !p.components().any(|c| c == Component::ParentDir)
            && p.starts_with(dir)
            && p != dir
    })
}

/// The checks of the team's records against the preset (pure): `agents` is the kernel's
/// `list_agents`, `dir` the team folder. Provider caching on, the git roots and the
/// worktree root under the team folder, the connectors' allowlists and the roles' limits.
pub fn check_records(agents: &[Value], p: &Preset, dir: &Path) -> Vec<Check> {
    let find = |id: &str| agents.iter().find(|a| a["id"] == id);
    let mut out = Vec::new();
    match find(&p.provider.id) {
        Some(r) if r["caching"] == CACHING => out.push(ok(format!(
            "provider `{}` caching is {CACHING}",
            p.provider.id
        ))),
        Some(r) => out.push(fail(
            format!("provider `{}` caching", p.provider.id),
            format!("{}, not {CACHING}", r["caching"]),
        )),
        None => out.push(fail(
            format!("provider `{}`", p.provider.id),
            "no such agent",
        )),
    }
    match find(LEAD) {
        Some(l)
            if l["provider_id"] == p.provider.id.as_str()
                && l["model"] == p.lead_model.as_str() =>
        {
            out.push(ok(format!(
                "lead runs `{}` on {}",
                p.provider.id, p.lead_model
            )));
        }
        Some(l) => out.push(fail(
            "lead provider and model",
            format!("provider_id {}, model {}", l["provider_id"], l["model"]),
        )),
        None => out.push(fail("lead", "no such agent")),
    }
    for c in &p.connectors {
        let what = format!("git `{}` roots are under the team folder", c.git_id);
        match find(&c.git_id) {
            Some(g) if is_under(dir, &g["repo_dir"]) && is_under(dir, &g["workspace_root"]) => {
                out.push(ok(what))
            }
            Some(g) => out.push(fail(
                what,
                format!(
                    "repo_dir {}, workspace_root {}",
                    g["repo_dir"], g["workspace_root"]
                ),
            )),
            None => out.push(fail(what, "no such agent")),
        }
        let what = format!("github `{}` allows only {}", c.github_id, c.repo);
        match find(&c.github_id) {
            Some(g) if g["repos"] == json!([c.repo]) && g["git"] == c.git_id.as_str() => {
                out.push(ok(what))
            }
            Some(g) => out.push(fail(
                what,
                format!("repos {}, git {}", g["repos"], g["git"]),
            )),
            None => out.push(fail(what, "no such agent")),
        }
    }
    let wt = p.worktrees.to_string_lossy().into_owned();
    let what = "the workers' files and exec are rooted at the worktree root".to_string();
    match (find(WORKER_FILES), find(WORKER_EXEC)) {
        (Some(f), Some(e))
            if f["root"] == wt.as_str() && e["cwd"] == wt.as_str() && is_under(dir, &f["root"]) =>
        {
            out.push(ok(what))
        }
        _ => out.push(fail(what, format!("not {wt}"))),
    }
    let what = format!("roles limit {} in all", p.total);
    match find(ROLES).map(|r| &r["limits"]) {
        Some(l) if l["total"] == p.total && l["per_role"] == json!(p.per_role) => {
            out.push(ok(what))
        }
        other => out.push(fail(what, format!("{other:?}"))),
    }
    out
}

/// `github whoami` of every connector: the app answers for the credential, with no model
/// called.
async fn whoami(kernel: &std::sync::Arc<fantastic_kernel::Kernel>, p: &Preset) -> Vec<Check> {
    let mut out = Vec::new();
    for c in &p.connectors {
        let what = format!("github `{}` whoami", c.github_id);
        let o = kernel
            .send(vec![Item::new(
                c.github_id.as_str(),
                json!({"type": "whoami"}),
            )])
            .await;
        out.push(match o.first() {
            Some(o) if matches!(o.status, Status::Completed) && o.result.get("error").is_none() => {
                ok(format!("{what}: app {}", o.result["app"]["slug"]))
            }
            Some(o) => fail(what, o.result.to_string()),
            None => fail(what, "no reply"),
        });
    }
    out
}

/// The dry check of the team folder `dir`: the preset parses, a fresh folder is applied,
/// the records agree with it and every github connector answers `whoami`. No model is
/// called.
pub async fn dry_check(dir: &Path) -> Result<Vec<Check>, String> {
    let preset = load(dir)?.ok_or_else(|| format!("{FILE}: not found in {}", dir.display()))?;
    let mut out = vec![ok(format!("{FILE} has every required field"))];
    let kernel = crate::start_all_offline(dir)
        .await
        .map_err(|e| e.to_string())?;
    let listed = kernel
        .send(vec![Item::new(
            DEFAULT_ROOT_ID,
            json!({"type": "list_agents"}),
        )])
        .await;
    let agents: Vec<Value> = listed
        .first()
        .and_then(|o| o.result.get("agents"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    out.extend(check_records(&agents, &preset, dir));
    out.extend(whoami(&kernel, &preset).await);
    kernel.shutdown().await;
    fantastic_sqlite_store::close_all(&kernel);
    bootstrap::shutdown(dir).map_err(|e| e.to_string())?;
    Ok(out)
}

/// `fantastic_team check`: print the dry check of the working directory and fail when any
/// line fails.
pub fn run_cli(args: &[String]) -> Result<(), String> {
    if let Some(a) = args.first() {
        return Err(format!(
            "check takes no argument (got `{a}`); it reads {FILE} in this folder"
        ));
    }
    let dir = std::env::current_dir().map_err(|e| e.to_string())?;
    // the fields are checked before anything is written
    load(&dir)?.ok_or_else(|| format!("{FILE}: not found in {}", dir.display()))?;
    apply(&dir)?;
    let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    let checks = rt.block_on(dry_check(&dir))?;
    let mut failed = 0;
    for c in &checks {
        match &c.failed {
            None => println!("ok   {}", c.what),
            Some(why) => {
                failed += 1;
                println!("FAIL {}: {why}", c.what);
            }
        }
    }
    if failed > 0 {
        return Err(format!("{failed} check(s) failed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fantastic_kernel::AgentId;

    /// The battle setup as a preset: two repos, the provider on Sonnet and Haiku.
    pub(crate) fn sample(dir: &Path) -> String {
        let d = dir.display();
        format!(
            r#"credentials:
  anthropic/api.anthropic.com:
    host: api.anthropic.com
    env: {{api_key: FANT_AK}}
  github/app:
    host: api.github.com
    env: {{app_id: FANT_GH_APP_ID, private_key: FANT_GH_APP_KEY}}
provider:
  endpoint: https://api.anthropic.com/v1/messages
  caching: provider_cache
  models: [claude-sonnet-5-5, claude-haiku-5-5]
  default_model: claude-sonnet-5-5
  context_window: 200000
  credentials: anthropic/api.anthropic.com
connectors:
  - repo: aisixteen/fantastic
    clone: {d}/repos/fantastic
    app: github/app
    author: {{name: "bot[bot]", email: bot@example.invalid}}
  - repo: aisixteen/fantastic_team
    clone: repos/fantastic_team
    app: github/app
    author: {{name: "bot[bot]", email: bot@example.invalid}}
    directive: Open draft pull requests only.
worktrees: wt
roles:
  limits: {{total: 10, per_role: {{lead: 2, worker: 8}}}}
lead:
  model: claude-sonnet-5-5
"#
        )
    }

    fn dir() -> tempfile::TempDir {
        tempfile::TempDir::new().unwrap()
    }

    #[test]
    fn a_whole_preset_parses_with_the_second_repo_getting_its_own_ids() {
        let d = dir();
        let p = parse(&sample(d.path()), d.path()).unwrap();
        let ids: Vec<(&str, &str)> = p
            .connectors
            .iter()
            .map(|c| (c.git_id.as_str(), c.github_id.as_str()))
            .collect();
        assert_eq!(
            ids,
            [
                ("git", "github"),
                ("git_fantastic_team", "github_fantastic_team")
            ]
        );
        assert_eq!(p.connectors[1].clone, d.path().join("repos/fantastic_team"));
        assert_eq!(p.connectors[1].remote, "origin");
        assert_eq!(p.worktrees, d.path().join("wt"));
        assert_eq!((p.total, p.per_role["worker"]), (10, 8));
        assert_eq!(p.provider.id, PROVIDER_ID);
    }

    #[test]
    fn a_missing_field_fails_naming_it() {
        let d = dir();
        for (cut, name) in [
            (
                "  endpoint: https://api.anthropic.com/v1/messages\n",
                "provider.endpoint",
            ),
            ("  caching: provider_cache\n", "provider.caching"),
            (
                "  credentials: anthropic/api.anthropic.com\n",
                "provider.credentials",
            ),
            ("worktrees: wt\n", "worktrees"),
            ("  model: claude-sonnet-5-5\n", "lead.model"),
            (
                "    author: {name: \"bot[bot]\", email: bot@example.invalid}\n",
                "connectors[0].author",
            ),
            ("    app: github/app\n", "connectors[0].app"),
        ] {
            let text = sample(d.path()).replacen(cut, "", 1);
            let err = parse(&text, d.path()).unwrap_err();
            assert!(
                err.contains(&format!("missing {name}")) || err.contains(&format!(", {name}")),
                "{name}: {err}"
            );
        }
        let err = parse("provider: {}\n", d.path()).unwrap_err();
        for name in [
            "credentials",
            "provider.endpoint",
            "provider.models",
            "connectors",
            "worktrees",
            "roles.limits",
            "lead",
        ] {
            assert!(err.contains(name), "{name}: {err}");
        }
    }

    #[test]
    fn what_is_declared_must_be_sound() {
        let d = dir();
        let s = sample(d.path());
        let bad = |from: &str, to: &str, want: &str| {
            let err = parse(&s.replacen(from, to, 1), d.path()).unwrap_err();
            assert!(err.contains(want), "{want}: {err}");
        };
        bad(
            "caching: provider_cache",
            "caching: none",
            "provider.caching must be provider_cache",
        );
        bad("FANT_AK}", "sk-ant-0123456789abcdef}", "never a value");
        bad(
            "clone: repos/fantastic_team",
            "clone: ../elsewhere",
            "connectors[1].clone",
        );
        bad("worktrees: wt", "worktrees: /tmp/wt", "worktrees:");
        bad(
            "repo: aisixteen/fantastic_team",
            "repo: aisixteen/fantastic",
            "repeats a repo",
        );
        bad(
            "default_model: claude-sonnet-5-5",
            "default_model: gpt",
            "default_model",
        );
        bad(
            "  model: claude-sonnet-5-5\n",
            "  model: nope\n",
            "lead.model `nope`",
        );
        bad("app: github/app", "app: github/other", "does not declare");
        bad(
            "worktrees: wt",
            "worktrees: wt\nsurprise: 1",
            "unknown field",
        );
    }

    /// The records of the blueprint with the preset in, flat.
    fn flat(dir: &Path) -> Vec<Value> {
        fn walk(list: &Value, out: &mut Vec<Value>) {
            for e in list.as_array().into_iter().flatten() {
                let mut rec = e.clone();
                rec.as_object_mut().unwrap().remove("agents");
                out.push(rec);
                walk(&e["agents"], out);
            }
        }
        let doc: Value =
            serde_yaml::from_str(&crate::hydrate::blueprint_text(dir).unwrap()).unwrap();
        let mut out = Vec::new();
        walk(&doc["agents"], &mut out);
        out
    }

    fn rec<'a>(all: &'a [Value], id: &str) -> &'a Value {
        all.iter()
            .find(|r| r["id"] == id)
            .unwrap_or_else(|| panic!("no {id}"))
    }

    #[test]
    fn the_preset_is_part_of_the_seeded_records() {
        let d = dir();
        std::fs::write(d.path().join(FILE), sample(d.path())).unwrap();
        let all = flat(d.path());
        let a = rec(&all, "anthropic");
        assert_eq!(a["caching"], "provider_cache");
        assert_eq!(
            a["models"],
            json!(["claude-sonnet-5-5", "claude-haiku-5-5"])
        );
        assert_eq!(
            a["credentials"],
            json!({"store": "credentials", "ref": "anthropic/api.anthropic.com"})
        );
        let c = rec(&all, "credentials");
        assert_eq!(
            c["refs"]["github/app"]["env"]["private_key"],
            "FANT_GH_APP_KEY"
        );
        assert_eq!(c["callable_by"]["github_fantastic_team"], json!(["get"]));
        let l = rec(&all, "lead");
        assert_eq!(
            (&l["provider_id"], &l["model"]),
            (&json!("anthropic"), &json!("claude-sonnet-5-5"))
        );
        assert_eq!(l["can_call"]["git_fantastic_team"], "*");
        assert_eq!(rec(&all, "github")["repos"], json!(["aisixteen/fantastic"]));
        let g2 = rec(&all, "github_fantastic_team");
        assert_eq!(
            (&g2["repos"], &g2["git"]),
            (
                &json!(["aisixteen/fantastic_team"]),
                &json!("git_fantastic_team")
            )
        );
        assert_eq!(g2["directive"], "Open draft pull requests only.");
        assert_eq!(
            rec(&all, "board")["repos"],
            json!(["aisixteen/fantastic", "aisixteen/fantastic_team"])
        );
        assert_eq!(
            rec(&all, "roles")["limits"],
            json!({"total": 10, "per_role": {"lead": 2, "worker": 8}})
        );
        let wt = d.path().join("wt").to_string_lossy().into_owned();
        assert_eq!(
            (
                &rec(&all, "worker_files")["root"],
                &rec(&all, "worker_exec")["cwd"]
            ),
            (&json!(wt), &json!(wt))
        );
        // no tab and no kernel in the preset's records
        assert!(all.iter().all(|r| r["handler_module"] != "ws_bridge"));
        // the whole record set on disk is written with nothing in it a key could be
        assert!(!serde_yaml::to_string(&all).unwrap().contains("sk-"));
    }

    #[test]
    fn a_folder_with_no_preset_seeds_the_plain_blueprint() {
        let d = dir();
        let all = flat(d.path());
        assert!(all
            .iter()
            .all(|r| r["id"] != "anthropic" && r["handler_module"] != "credential_store.tools"));
        assert_eq!(rec(&all, "github")["repos"], json!(["owner/name"]));
    }

    #[test]
    fn the_checks_hold_for_the_seeded_records_and_name_what_is_off() {
        let d = dir();
        std::fs::write(d.path().join(FILE), sample(d.path())).unwrap();
        let p = load(d.path()).unwrap().unwrap();
        let mut all = flat(d.path());
        let checks = check_records(&all, &p, d.path());
        assert!(checks.iter().all(|c| c.failed.is_none()), "{checks:?}");
        assert_eq!(checks.len(), 8);
        for r in all.iter_mut() {
            if r["id"] == "anthropic" {
                r["caching"] = json!("none");
            }
            if r["id"] == "git" {
                r["repo_dir"] = json!("/elsewhere/fantastic");
            }
        }
        let bad: Vec<String> = check_records(&all, &p, d.path())
            .into_iter()
            .filter(|c| c.failed.is_some())
            .map(|c| c.what)
            .collect();
        assert_eq!(bad.len(), 2, "{bad:?}");
        assert!(
            bad[0].contains("caching") && bad[1].contains("`git`"),
            "{bad:?}"
        );
    }

    #[test]
    fn the_shipped_example_is_a_whole_preset_with_no_value_in_it() {
        let text = include_str!("../../../examples/team.yaml");
        let d = dir();
        let p = parse(text, d.path()).unwrap();
        assert_eq!(p.connectors.len(), 2);
        assert_eq!(p.provider.caching, CACHING);
        assert!(!text.contains("sk-"));
    }

    #[test]
    fn a_bad_preset_writes_nothing() {
        let d = dir();
        std::fs::write(d.path().join(FILE), "provider: {}\n").unwrap();
        let err = apply(d.path()).unwrap_err();
        assert!(
            err.contains("missing") && err.contains("provider.endpoint"),
            "{err}"
        );
        assert!(!d.path().join(".fantastic").exists());
    }

    #[test]
    fn the_first_start_keeps_the_default_model_and_a_started_folder_is_left_alone() {
        let d = dir();
        // a clone for the git connectors' roots is not needed: nothing is asked of git here
        std::fs::write(d.path().join(FILE), sample(d.path())).unwrap();
        assert!(apply(d.path()).unwrap());
        assert!(d.path().join(".fantastic/memory.db").exists());
        assert!(!apply(d.path()).unwrap(), "a second start applies nothing");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let got = rt.block_on(async {
            let k = crate::start_all_offline(d.path()).await.unwrap();
            let m = k
                .send(vec![Item::new(
                    AgentId::from("anthropic"),
                    json!({"type": "models"}),
                )])
                .await;
            k.shutdown().await;
            fantastic_sqlite_store::close_all(&k);
            bootstrap::shutdown(d.path()).unwrap();
            m[0].result.clone()
        });
        assert_eq!(got["default"], "claude-sonnet-5-5", "{got}");
    }
}
