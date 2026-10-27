//! What `team.yaml` holds: the agent templates an agent may start agents
//! from, the connectors and their directives, the queue's id, and the
//! credential store. It does not hold how many agents run (the first agent's
//! instructions say that), nor any provider or model name in code.
//!
//! The caller defines the agents: a file with no templates and no `first_agent`
//! is valid and starts only the system agents. `first_agent` (the lead) is
//! optional; `roles:` needs it, because the lead spawns through the registry.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
use serde::Deserialize;
use serde_json::{json, Map, Value};

/// The file name the team looks for in its directory.
pub const FILE_NAME: &str = "team.yaml";

/// Keys a template's `extra` may not set.
pub const RESERVED: [&str; 12] = [
    "type",
    "id",
    "parent_id",
    "handler_module",
    "model",
    "endpoint",
    "credentials",
    "price_input_per_mtok",
    "price_output_per_mtok",
    "memory_store_id",
    "can_call",
    "callable_by",
];

/// Keys a connector's `record` may not set.
pub const CONNECTOR_RESERVED: [&str; 4] = ["type", "id", "parent_id", "handler_module"];

/// The whole file.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamConfig {
    /// The agent templates, by name. Absent: none.
    #[serde(default)]
    pub templates: BTreeMap<String, Template>,
    /// The agent the binary creates and instructs. Absent: no agent is created.
    #[serde(default)]
    pub first_agent: Option<FirstAgent>,
    /// The connector agents, by id.
    #[serde(default)]
    pub connectors: BTreeMap<String, Connector>,
    /// Where the queue lives. Absent: the agent `queue`.
    #[serde(default = "default_queue")]
    pub queue: QueueRef,
    /// The credential store the team creates when it has none.
    #[serde(default)]
    pub credentials: Option<CredentialStore>,
    /// The team's groups, by name: an optional AI driver and the worker ids in it.
    #[serde(default)]
    pub groups: BTreeMap<String, GroupCfg>,
    /// The board: the repos it caches and the GitHub connector that reads them.
    #[serde(default)]
    pub board: Option<BoardCfg>,
    /// The roles registry (fantastic's `agentic_resources`): the card folders and
    /// the default limits. Absent: the team has no registry.
    #[serde(default)]
    pub roles: Option<RolesCfg>,
}

/// The roles section: where the registry lives, which folders hold the role cards,
/// and the limits set on every start.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RolesCfg {
    /// The registry agent's id.
    #[serde(default = "default_roles_id")]
    pub id: String,
    /// The folders of role cards, relative to the team's directory.
    #[serde(default = "default_role_dirs")]
    pub dirs: Vec<String>,
    /// The default limits, set with `set_limit` at every start.
    #[serde(default)]
    pub limits: RoleLimits,
}

/// The registry's limits: agents running in total, and per role.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleLimits {
    /// Agents running at once, all roles together. Absent: no cap.
    #[serde(default)]
    pub total: Option<u64>,
    /// Agents running at once per role name.
    #[serde(default)]
    pub per_role: BTreeMap<String, u64>,
}

/// A group of the team: its AI driver (none when absent) and its members.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupCfg {
    /// The agent that receives the group's digest. Absent for a group with no AI.
    #[serde(default)]
    pub driver: Option<String>,
    /// The worker ids in the group.
    #[serde(default)]
    pub members: Vec<String>,
    /// Seconds between the cadencer's digests for this group. Unset: the default.
    #[serde(default)]
    pub cadence: Option<u64>,
    /// Seconds a member holding a task may sit still before it is flagged. Unset: the default.
    #[serde(default)]
    pub stall_after: Option<u64>,
    /// Quiet mode for this group: send only changes. Unset: the default.
    #[serde(default)]
    pub quiet: Option<bool>,
    /// Whether the cadencer digests this group at all. Unset: the default (on).
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// The board section: what it caches and through which connector.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardCfg {
    /// The repos to cache, `owner/name`.
    pub repos: Vec<String>,
    /// The GitHub connector's id (a connector of this file).
    pub github: String,
}

/// An agent template: the record fields of an AI agent, as data.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    /// The backend's handler module.
    pub handler_module: String,
    /// The model the backend calls.
    pub model: String,
    /// The endpoint the backend calls.
    pub endpoint: String,
    /// The credential store ref the backend reads its key from.
    #[serde(default)]
    pub credentials: Option<CredRef>,
    /// The price of one million input tokens.
    #[serde(default)]
    pub price_input_per_mtok: Option<f64>,
    /// The price of one million output tokens.
    #[serde(default)]
    pub price_output_per_mtok: Option<f64>,
    /// Agent ids an agent of this template may call.
    #[serde(default)]
    pub can_call: Vec<String>,
    /// Any other record fields, passed through as is.
    #[serde(default)]
    pub extra: Map<String, Value>,
}

/// A credential store ref on a record.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredRef {
    /// The credential store's agent id.
    pub store: String,
    /// The ref inside that store.
    #[serde(rename = "ref")]
    pub ref_: String,
}

/// The agent the binary creates and instructs.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirstAgent {
    /// The first agent's id.
    #[serde(default = "default_first_id")]
    pub id: String,
    /// The name of the template it is made from.
    pub template: String,
    /// The file with its instructions, relative to the config file's folder.
    #[serde(default)]
    pub directive_file: Option<String>,
}

/// A connector agent and what agents may do with it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connector {
    /// Its handler module.
    pub kind: String,
    /// Its record fields.
    #[serde(default)]
    pub record: Map<String, Value>,
    /// What agents may do with it, inline.
    #[serde(default)]
    pub directive: Option<String>,
    /// What agents may do with it, read from a file relative to the config file's folder.
    #[serde(default)]
    pub directive_file: Option<String>,
}

/// Where the queue lives.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueRef {
    /// The queue agent's id.
    pub id: String,
}

/// The credential store the team creates when it has none.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialStore {
    /// The store's agent id.
    #[serde(default = "default_store_id")]
    pub id: String,
    /// The backend handler, e.g. `env`.
    pub backend: String,
    /// The refs the store knows.
    #[serde(default)]
    pub refs: Map<String, Value>,
}

fn default_queue() -> QueueRef {
    QueueRef { id: "queue".into() }
}

impl TeamConfig {
    /// The first agent's id, when the file defines one.
    pub fn lead_id(&self) -> Option<&str> {
        self.first_agent.as_ref().map(|f| f.id.as_str())
    }
}

fn default_first_id() -> String {
    "lead".into()
}

fn default_store_id() -> String {
    "credentials".into()
}

/// The task registry's agent id: a team with `roles:` has one, and the id is taken.
pub const TASKS_ID: &str = "tasks";

fn default_roles_id() -> String {
    "roles".into()
}

fn default_role_dirs() -> Vec<String> {
    vec!["roles".into()]
}

/// Parse and validate a `team.yaml` text.
pub fn parse(text: &str) -> Result<TeamConfig, String> {
    let v = fantastic_kernel::yaml_io::load(text).map_err(|e| format!("not YAML — {e}"))?;
    let cfg: TeamConfig = serde_json::from_value(v).map_err(|e| e.to_string())?;
    validate(&cfg)?;
    Ok(cfg)
}

/// Read, parse and validate the file at `path`.
pub fn load(path: &Path) -> Result<TeamConfig, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Check the cross-field rules; the first failure is the error.
pub fn validate(cfg: &TeamConfig) -> Result<(), String> {
    for (n, t) in &cfg.templates {
        for (field, val) in [
            ("handler_module", &t.handler_module),
            ("model", &t.model),
            ("endpoint", &t.endpoint),
        ] {
            if val.trim().is_empty() {
                return Err(format!("template `{n}`: {field} is empty"));
            }
        }
        for (field, price) in [
            ("price_input_per_mtok", t.price_input_per_mtok),
            ("price_output_per_mtok", t.price_output_per_mtok),
        ] {
            if let Some(p) = price {
                if !p.is_finite() || p < 0.0 {
                    return Err(format!("template `{n}`: {field} must be a number >= 0"));
                }
            }
        }
        if let Some(r) = &t.credentials {
            if r.store.trim().is_empty() || r.ref_.trim().is_empty() {
                return Err(format!("template `{n}`: credentials needs store and ref"));
            }
        }
        for key in t.extra.keys() {
            if RESERVED.contains(&key.as_str()) {
                return Err(format!(
                    "template `{n}`: extra.{key} is reserved — use the template's own field, or let the team set it"
                ));
            }
        }
        if t.can_call.iter().any(|c| c.trim().is_empty()) {
            return Err(format!("template `{n}`: can_call has an empty id"));
        }
    }
    if let Some(f) = &cfg.first_agent {
        if f.id.trim().is_empty() {
            return Err("first_agent.id is empty".into());
        }
        if !cfg.templates.contains_key(&f.template) {
            let names: Vec<&str> = cfg.templates.keys().map(String::as_str).collect();
            return Err(format!(
                "first_agent.template `{}` is not a template (have: {})",
                f.template,
                names.join(", ")
            ));
        }
    }
    if cfg.roles.is_some() && cfg.first_agent.is_none() {
        return Err("roles needs first_agent: the lead spawns through the registry".into());
    }
    if cfg.roles.is_some() && cfg.templates.is_empty() {
        return Err("roles needs templates: the registry's tiers are the templates".into());
    }
    if cfg.queue.id.trim().is_empty() {
        return Err("queue.id is empty".into());
    }
    for (c, conn) in &cfg.connectors {
        if conn.kind.trim().is_empty() {
            return Err(format!("connector `{c}`: kind is empty"));
        }
        if conn.directive.is_some() && conn.directive_file.is_some() {
            return Err(format!(
                "connector `{c}`: give directive or directive_file, not both"
            ));
        }
        for key in conn.record.keys() {
            if CONNECTOR_RESERVED.contains(&key.as_str()) {
                return Err(format!(
                    "connector `{c}`: record.{key} is reserved — the team sets it"
                ));
            }
        }
    }
    if let Some(s) = &cfg.credentials {
        if s.id.trim().is_empty() || s.backend.trim().is_empty() {
            return Err("credentials: id and backend are required".into());
        }
        for (n, t) in &cfg.templates {
            if let Some(r) = &t.credentials {
                if r.store != s.id {
                    return Err(format!(
                        "template `{n}`: credentials.store `{}` is not the configured store `{}`",
                        r.store, s.id
                    ));
                }
            }
        }
    }
    for (g, grp) in &cfg.groups {
        if g.trim().is_empty() {
            return Err("group name is empty".into());
        }
        if cfg.lead_id() == Some(g.as_str()) {
            return Err(format!("group `{g}`: the name is the first agent's id"));
        }
        if grp.driver.as_deref().is_some_and(|d| d.trim().is_empty()) {
            return Err(format!("group `{g}`: driver is empty"));
        }
        if grp.members.iter().any(|m| m.trim().is_empty()) {
            return Err(format!("group `{g}`: a member id is empty"));
        }
        if grp.cadence == Some(0) {
            return Err(format!(
                "group `{g}`: cadence must be a whole number of seconds >= 1"
            ));
        }
        if grp.stall_after == Some(0) {
            return Err(format!(
                "group `{g}`: stall_after must be a whole number of seconds >= 1"
            ));
        }
    }
    if let Some(b) = &cfg.board {
        if !cfg.connectors.contains_key(&b.github) {
            return Err(format!("board: github `{}` is not a connector", b.github));
        }
        if b.repos.is_empty() {
            return Err("board: repos needs at least one owner/name".into());
        }
        for r in &b.repos {
            let parts: Vec<&str> = r.split('/').collect();
            let ok = parts.len() == 2 && parts.iter().all(|p| !p.trim().is_empty());
            if !ok {
                return Err(format!("board: repo `{r}` is not owner/name"));
            }
        }
    }
    if let Some(r) = &cfg.roles {
        if r.id.trim().is_empty() {
            return Err("roles.id is empty".into());
        }
        if r.dirs.is_empty() {
            return Err("roles.dirs is empty — name at least one folder of role cards".into());
        }
        if r.dirs.iter().any(|d| d.trim().is_empty()) {
            return Err("roles.dirs has an empty folder".into());
        }
        if r.limits.per_role.keys().any(|k| k.trim().is_empty()) {
            return Err("roles.limits.per_role has an empty role name".into());
        }
    }
    let mut ids: Vec<&str> = vec![cfg.queue.id.as_str()];
    ids.extend(cfg.lead_id());
    if let Some(r) = &cfg.roles {
        ids.push(r.id.as_str());
        ids.push(TASKS_ID);
    }
    ids.extend(cfg.connectors.keys().map(String::as_str));
    if let Some(s) = &cfg.credentials {
        ids.push(s.id.as_str());
    }
    let mut seen: HashSet<&str> = HashSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(format!("agent id `{id}` is used twice"));
        }
    }
    Ok(())
}

/// The cadencer settings the groups set in `team.yaml`: group name → the keys
/// that group sets (`cadence`, `stall_after`, `quiet`, `enabled`). Unset keys
/// are left out, and a group that sets none is left out: it follows the default.
pub fn group_settings(cfg: &TeamConfig) -> Value {
    let mut out = Map::new();
    for (g, grp) in &cfg.groups {
        let mut m = Map::new();
        if let Some(v) = grp.cadence {
            m.insert("cadence".into(), json!(v));
        }
        if let Some(v) = grp.stall_after {
            m.insert("stall_after".into(), json!(v));
        }
        if let Some(v) = grp.quiet {
            m.insert("quiet".into(), json!(v));
        }
        if let Some(v) = grp.enabled {
            m.insert("enabled".into(), json!(v));
        }
        if !m.is_empty() {
            out.insert(g.clone(), Value::Object(m));
        }
    }
    Value::Object(out)
}

/// The memory store id for an agent.
pub fn memory_id(agent_id: &str) -> String {
    format!("{agent_id}_memory")
}

/// A fantastic `create_agent` payload for an agent made from `tpl`.
pub fn agent_record(tpl: &Template, id: &str) -> Value {
    let mem = memory_id(id);
    let mut m = Map::new();
    m.insert("type".into(), json!("create_agent"));
    m.insert("parent_id".into(), json!(DEFAULT_ROOT_ID));
    m.insert("id".into(), json!(id));
    m.insert("handler_module".into(), json!(tpl.handler_module));
    m.insert("model".into(), json!(tpl.model));
    m.insert("endpoint".into(), json!(tpl.endpoint));
    if let Some(r) = &tpl.credentials {
        m.insert(
            "credentials".into(),
            json!({"store": r.store, "ref": r.ref_}),
        );
    }
    if let Some(p) = tpl.price_input_per_mtok {
        m.insert("price_input_per_mtok".into(), json!(p));
    }
    if let Some(p) = tpl.price_output_per_mtok {
        m.insert("price_output_per_mtok".into(), json!(p));
    }
    m.insert("memory_store_id".into(), json!(&mem));
    let mut can_call = Map::new();
    can_call.insert("$self".into(), json!("*"));
    can_call.insert("$subtree".into(), json!("*"));
    can_call.insert(mem.clone(), json!("*"));
    for c in &tpl.can_call {
        can_call.insert(c.clone(), json!("*"));
    }
    m.insert("can_call".into(), Value::Object(can_call));
    m.insert("callable_by".into(), json!({"$self": "*", "$parent": "*"}));
    for (k, v) in &tpl.extra {
        m.insert(k.clone(), v.clone());
    }
    Value::Object(m)
}

/// The tier `tpl` stands for in the roles registry's `tiers`: the backend fields
/// of the template (handler module, model, endpoint, credentials, prices and the
/// `extra` fields) plus the shared memory store. No `can_call` (the role card
/// grants), no id, no parent. The tier's KEY is the template's name.
pub fn tier_fields(tpl: &Template, memory_store: &str) -> Value {
    let mut m = Map::new();
    m.insert("handler_module".into(), json!(tpl.handler_module));
    m.insert("model".into(), json!(tpl.model));
    m.insert("endpoint".into(), json!(tpl.endpoint));
    if let Some(r) = &tpl.credentials {
        m.insert(
            "credentials".into(),
            json!({"store": r.store, "ref": r.ref_}),
        );
    }
    if let Some(p) = tpl.price_input_per_mtok {
        m.insert("price_input_per_mtok".into(), json!(p));
    }
    if let Some(p) = tpl.price_output_per_mtok {
        m.insert("price_output_per_mtok".into(), json!(p));
    }
    m.insert("memory_store_id".into(), json!(memory_store));
    for (k, v) in &tpl.extra {
        m.insert(k.clone(), v.clone());
    }
    Value::Object(m)
}

/// A `create_agent` payload for the memory store of `agent_id`.
pub fn memory_record(agent_id: &str) -> Value {
    let mut callable_by = Map::new();
    callable_by.insert("$self".into(), json!("*"));
    callable_by.insert(agent_id.into(), json!("*"));
    json!({
        "type": "create_agent",
        "parent_id": DEFAULT_ROOT_ID,
        "id": memory_id(agent_id),
        "handler_module": "ram_store.tools",
        "callable_by": Value::Object(callable_by),
    })
}

/// A `create_agent` payload for the credential store.
pub fn store_record(s: &CredentialStore) -> Value {
    json!({
        "type": "create_agent",
        "parent_id": DEFAULT_ROOT_ID,
        "id": s.id,
        "handler_module": "credential_store.tools",
        "backend": s.backend,
        "refs": s.refs,
    })
}

/// A `create_agent` payload for a connector called `id`.
pub fn connector_record(id: &str, c: &Connector) -> Value {
    let mut m = Map::new();
    m.insert("type".into(), json!("create_agent"));
    m.insert("parent_id".into(), json!(DEFAULT_ROOT_ID));
    m.insert("id".into(), json!(id));
    m.insert("handler_module".into(), json!(c.kind));
    for (k, v) in &c.record {
        m.insert(k.clone(), v.clone());
    }
    Value::Object(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: &str = "templates:\n  t:\n    handler_module: h.tools\n    model: m\n    endpoint: http://e\nfirst_agent:\n  template: t\nqueue:\n  id: q\n";

    fn tpl(extra: Value) -> Template {
        Template {
            handler_module: "x_backend.tools".into(),
            model: "m-small".into(),
            endpoint: "http://127.0.0.1:9/v1".into(),
            credentials: Some(CredRef {
                store: "credentials".into(),
                ref_: "x/127.0.0.1".into(),
            }),
            price_input_per_mtok: Some(0.1),
            price_output_per_mtok: Some(0.5),
            can_call: vec!["exec".into(), "queue".into()],
            extra: extra.as_object().cloned().unwrap_or_default(),
        }
    }

    #[test]
    fn the_example_parses() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/team.yaml");
        let cfg = load(&path).unwrap();
        let keys: Vec<&str> = cfg.templates.keys().map(String::as_str).collect();
        assert_eq!(keys, ["issue_keeper", "local", "small", "strong"]);
        let first = cfg.first_agent.as_ref().unwrap();
        assert_eq!(first.id, "lead");
        assert_eq!(first.template, "strong");
        assert_eq!(first.directive_file.as_deref(), Some("roles/lead.md"));
        assert_eq!(cfg.templates["small"].price_input_per_mtok, Some(0.1));
        assert_eq!(
            cfg.templates["small"].credentials.as_ref().unwrap().ref_,
            "anthropic/api.anthropic.com"
        );
        assert_eq!(cfg.templates["strong"].extra["max_tokens"], json!(32000));
        assert_eq!(cfg.connectors["git"].kind, "team_git.tools");
        assert_eq!(cfg.connectors["git"].record["remote"], json!("origin"));
        assert_eq!(cfg.queue.id, "queue");
        assert_eq!(cfg.credentials.as_ref().unwrap().backend, "env");
    }

    #[test]
    fn group_cadencer_settings_parse() {
        let y = format!("{MIN}groups:\n  g:\n    members: [a]\n    cadence: 600\n    stall_after: 900\n    quiet: false\n    enabled: false\n");
        let cfg = parse(&y).unwrap();
        let g = &cfg.groups["g"];
        assert_eq!(g.cadence, Some(600));
        assert_eq!(g.stall_after, Some(900));
        assert_eq!(g.quiet, Some(false));
        assert_eq!(g.enabled, Some(false));
        assert_eq!(
            group_settings(&cfg),
            json!({"g": {"cadence": 600, "stall_after": 900, "quiet": false, "enabled": false}})
        );
    }

    #[test]
    fn group_cadencer_settings_default_to_none() {
        let cfg = parse(&format!("{MIN}groups:\n  g:\n    members: [a]\n")).unwrap();
        let g = &cfg.groups["g"];
        assert_eq!(
            (g.cadence, g.stall_after, g.quiet, g.enabled),
            (None, None, None, None)
        );
        assert_eq!(group_settings(&cfg), json!({}));
    }

    #[test]
    fn group_cadencer_settings_reject_bad_values() {
        let with = |kv: &str| format!("{MIN}groups:\n  g:\n    members: [a]\n    {kv}\n");
        let e = parse(&with("cadence: 0")).unwrap_err();
        assert_eq!(
            e,
            "group `g`: cadence must be a whole number of seconds >= 1"
        );
        let e = parse(&with("stall_after: 0")).unwrap_err();
        assert_eq!(
            e,
            "group `g`: stall_after must be a whole number of seconds >= 1"
        );
        assert!(parse(&with("cadence: -5")).is_err());
        assert!(parse(&with("cadence: x")).is_err());
        assert!(parse(&with("quiet: 3")).is_err());
    }

    #[test]
    fn no_example_template_grants_exec_or_files() {
        let cfg = parse(include_str!("../../../examples/team.yaml")).unwrap();
        for (n, t) in &cfg.templates {
            for c in &t.can_call {
                assert!(
                    c != "exec" && c != "files",
                    "template `{n}` grants the unrestricted agent `{c}`"
                );
            }
        }
    }

    #[test]
    fn a_minimal_file_parses_with_defaults() {
        let cfg = parse(MIN).unwrap();
        let first = cfg.first_agent.as_ref().unwrap();
        assert_eq!(first.id, "lead");
        assert_eq!(first.directive_file, None);
        assert!(cfg.connectors.is_empty());
        assert_eq!(cfg.credentials, None);
        let t = &cfg.templates["t"];
        assert!(t.can_call.is_empty());
        assert!(t.extra.is_empty());
        assert_eq!(t.price_input_per_mtok, None);
        assert_eq!(t.price_output_per_mtok, None);
    }

    #[test]
    fn unknown_keys_are_refused() {
        let e = parse(&format!("{MIN}colour: red\n")).unwrap_err();
        assert!(e.contains("unknown field `colour`"));
        let e = parse(&format!("{MIN}workers: 4\n")).unwrap_err();
        assert!(e.contains("unknown field `workers`"));
        let e = parse(&MIN.replace(
            "    endpoint: http://e\n",
            "    endpoint: http://e\n    count: 3\n",
        ))
        .unwrap_err();
        assert!(e.contains("unknown field `count`"));
        let e = parse(&MIN.replace("  id: q\n", "  id: q\n  extra_thing: 1\n")).unwrap_err();
        assert!(e.contains("unknown field `extra_thing`"));
    }

    #[test]
    fn empty_config_is_valid_and_defines_no_agent() {
        for text in ["{}", "templates: {}\n"] {
            let cfg = parse(text).unwrap();
            assert!(cfg.first_agent.is_none());
            assert!(cfg.templates.is_empty());
            assert!(cfg.roles.is_none());
            assert_eq!(cfg.queue.id, "queue");
            assert_eq!(cfg.lead_id(), None);
        }
    }

    #[test]
    fn first_agent_is_optional_with_templates_only() {
        let text = MIN.replace("first_agent:\n  template: t\n", "");
        let cfg = parse(&text).unwrap();
        assert!(cfg.first_agent.is_none());
        assert_eq!(cfg.templates.len(), 1);
        assert_eq!(cfg.queue.id, "q");
    }

    #[test]
    fn roles_without_first_agent_is_refused() {
        let text = MIN.replace("first_agent:\n  template: t\n", "") + "roles: {}\n";
        let e = parse(&text).unwrap_err();
        assert!(e.contains("first_agent"), "{e}");
        let e = parse("roles: {}\n").unwrap_err();
        assert!(e.contains("first_agent"), "{e}");
    }

    #[test]
    fn a_group_may_not_reuse_nothing_when_no_lead() {
        let cfg = parse("groups:\n  lead:\n    members: [w1]\n").unwrap();
        assert!(cfg.first_agent.is_none());
        let e = parse(&(MIN.to_string() + "groups:\n  lead:\n    members: [w1]\n")).unwrap_err();
        assert_eq!(e, "group `lead`: the name is the first agent's id");
    }

    #[test]
    fn worker_only_config_parses() {
        let text = "templates:\n  worker:\n    handler_module: h.tools\n    model: m\n    endpoint: http://e\nfirst_agent:\n  id: w1\n  template: worker\n";
        let cfg = parse(text).unwrap();
        let first = cfg.first_agent.as_ref().unwrap();
        assert_eq!(first.id, "w1");
        assert_eq!(first.directive_file, None);
        assert!(cfg.roles.is_none());
        assert!(cfg.groups.is_empty());
        assert_eq!(cfg.lead_id(), Some("w1"));
    }

    #[test]
    fn a_missing_template_is_refused() {
        let e = parse(&MIN.replace("  template: t\n", "  template: big\n")).unwrap_err();
        assert_eq!(e, "first_agent.template `big` is not a template (have: t)");
    }

    #[test]
    fn missing_required_fields_are_refused() {
        let cfg = parse(&MIN.replace("queue:\n  id: q\n", "")).unwrap();
        assert_eq!(cfg.queue.id, "queue", "an absent queue is the default one");
        let e = parse(&MIN.replace("    model: m\n", "")).unwrap_err();
        assert!(e.contains("missing field `model`"));
    }

    #[test]
    fn validate_texts_are_exact() {
        let base = parse(MIN).unwrap();

        let mut cfg = base.clone();
        cfg.templates.clear();
        assert_eq!(
            validate(&cfg).unwrap_err(),
            "first_agent.template `t` is not a template (have: )"
        );

        let mut cfg = base.clone();
        cfg.templates.get_mut("t").unwrap().model = " ".into();
        assert_eq!(validate(&cfg).unwrap_err(), "template `t`: model is empty");

        let mut cfg = base.clone();
        cfg.templates.get_mut("t").unwrap().price_output_per_mtok = Some(-1.0);
        assert_eq!(
            validate(&cfg).unwrap_err(),
            "template `t`: price_output_per_mtok must be a number >= 0"
        );

        let mut cfg = base.clone();
        cfg.templates
            .get_mut("t")
            .unwrap()
            .extra
            .insert("memory_store_id".into(), json!("x"));
        assert_eq!(
            validate(&cfg).unwrap_err(),
            "template `t`: extra.memory_store_id is reserved — use the template's own field, or let the team set it"
        );

        let mut cfg = base.clone();
        cfg.connectors.insert(
            "c".into(),
            Connector {
                kind: "k.tools".into(),
                record: Map::new(),
                directive: Some("d".into()),
                directive_file: Some("d.md".into()),
            },
        );
        assert_eq!(
            validate(&cfg).unwrap_err(),
            "connector `c`: give directive or directive_file, not both"
        );

        let mut cfg = base.clone();
        cfg.connectors.insert(
            "c".into(),
            Connector {
                kind: "k.tools".into(),
                record: json!({"id": "x"}).as_object().unwrap().clone(),
                directive: None,
                directive_file: None,
            },
        );
        assert_eq!(
            validate(&cfg).unwrap_err(),
            "connector `c`: record.id is reserved — the team sets it"
        );

        let mut cfg = base.clone();
        cfg.templates.get_mut("t").unwrap().credentials = Some(CredRef {
            store: "credentials".into(),
            ref_: "a/b".into(),
        });
        cfg.credentials = Some(CredentialStore {
            id: "creds".into(),
            backend: "env".into(),
            refs: Map::new(),
        });
        assert_eq!(
            validate(&cfg).unwrap_err(),
            "template `t`: credentials.store `credentials` is not the configured store `creds`"
        );

        let mut cfg = base.clone();
        cfg.connectors.insert(
            "q".into(),
            Connector {
                kind: "k.tools".into(),
                record: Map::new(),
                directive: None,
                directive_file: None,
            },
        );
        assert_eq!(validate(&cfg).unwrap_err(), "agent id `q` is used twice");
    }

    #[test]
    fn agent_record_is_exact() {
        let r = agent_record(&tpl(json!({"max_tokens": 8000})), "w1");
        assert_eq!(
            r,
            json!({
                "type": "create_agent",
                "parent_id": "kernel_state",
                "id": "w1",
                "handler_module": "x_backend.tools",
                "model": "m-small",
                "endpoint": "http://127.0.0.1:9/v1",
                "credentials": {"store": "credentials", "ref": "x/127.0.0.1"},
                "price_input_per_mtok": 0.1,
                "price_output_per_mtok": 0.5,
                "memory_store_id": "w1_memory",
                "can_call": {"$self": "*", "$subtree": "*", "w1_memory": "*", "exec": "*", "queue": "*"},
                "callable_by": {"$self": "*", "$parent": "*"},
                "max_tokens": 8000
            })
        );
    }

    #[test]
    fn agent_record_leaves_out_what_is_unset() {
        let mut t = tpl(json!({}));
        t.credentials = None;
        t.price_input_per_mtok = None;
        t.price_output_per_mtok = None;
        t.can_call.clear();
        let r = agent_record(&t, "w2");
        assert!(r.get("credentials").is_none());
        assert!(r.get("price_input_per_mtok").is_none());
        assert!(r.get("price_output_per_mtok").is_none());
        assert_eq!(
            r["can_call"],
            json!({"$self": "*", "$subtree": "*", "w2_memory": "*"})
        );
    }

    #[test]
    fn memory_store_and_connector_records_are_exact() {
        assert_eq!(
            memory_record("w1"),
            json!({
                "type": "create_agent",
                "parent_id": "kernel_state",
                "id": "w1_memory",
                "handler_module": "ram_store.tools",
                "callable_by": {"$self": "*", "w1": "*"}
            })
        );
        let s = CredentialStore {
            id: "credentials".into(),
            backend: "env".into(),
            refs: json!({"a/h": {"host": "h"}}).as_object().unwrap().clone(),
        };
        assert_eq!(
            store_record(&s),
            json!({
                "type": "create_agent",
                "parent_id": "kernel_state",
                "id": "credentials",
                "handler_module": "credential_store.tools",
                "backend": "env",
                "refs": {"a/h": {"host": "h"}}
            })
        );
        let c = Connector {
            kind: "team_git.tools".into(),
            record: json!({"remote": "origin"}).as_object().unwrap().clone(),
            directive: None,
            directive_file: None,
        };
        assert_eq!(
            connector_record("git", &c),
            json!({
                "type": "create_agent",
                "parent_id": "kernel_state",
                "id": "git",
                "handler_module": "team_git.tools",
                "remote": "origin"
            })
        );
    }

    #[test]
    fn a_missing_file_is_named() {
        let e = load(Path::new("/nonexistent/team.yaml")).unwrap_err();
        assert!(e.starts_with("cannot read /nonexistent/team.yaml: "));
    }

    #[test]
    fn groups_reject_empty_member_and_unknown_keys() {
        let g = |body: &str| parse(&format!("{MIN}groups:\n  a:\n{body}"));
        let e = g("    members: [\"\"]\n").unwrap_err();
        assert_eq!(e, "group `a`: a member id is empty");
        let e = g("    driver: \"\"\n").unwrap_err();
        assert_eq!(e, "group `a`: driver is empty");
        let e = g("    members: [w1]\n    colour: red\n").unwrap_err();
        assert!(e.contains("unknown field `colour`"), "{e}");
        let e = parse(&format!("{MIN}groups:\n  lead:\n    members: [w1]\n")).unwrap_err();
        assert_eq!(e, "group `lead`: the name is the first agent's id");
        let ok = parse(&format!(
            "{MIN}groups:\n  a:\n    driver: d\n    members: [w1]\n"
        ))
        .unwrap();
        assert_eq!(ok.groups["a"].driver.as_deref(), Some("d"));
        assert_eq!(ok.groups["a"].members, ["w1"]);
    }

    #[test]
    fn board_names_a_known_connector_and_repos() {
        let with = |board: &str| {
            parse(&format!(
                "{MIN}connectors:\n  git:\n    kind: team_github.tools\nboard:\n{board}"
            ))
        };
        let ok = with("  repos: [o/r]\n  github: git\n").unwrap();
        assert_eq!(ok.board.as_ref().unwrap().repos, ["o/r"]);
        let e = with("  repos: [o/r]\n  github: gh\n").unwrap_err();
        assert_eq!(e, "board: github `gh` is not a connector");
        let e = with("  repos: []\n  github: git\n").unwrap_err();
        assert_eq!(e, "board: repos needs at least one owner/name");
        let e = with("  repos: [o]\n  github: git\n").unwrap_err();
        assert_eq!(e, "board: repo `o` is not owner/name");
        let e = with("  repos: [o/r]\n  github: git\n  ask_to: me\n").unwrap_err();
        assert!(e.contains("unknown field `ask_to`"), "{e}");
    }

    #[test]
    fn roles_section_parses_with_defaults() {
        let cfg = parse(&format!("{MIN}roles: {{}}\n")).unwrap();
        let r = cfg.roles.unwrap();
        assert_eq!(r.id, "roles");
        assert_eq!(r.dirs, ["roles"]);
        assert_eq!(r.limits, RoleLimits::default());
        assert_eq!(r.limits.total, None);
        assert!(r.limits.per_role.is_empty());
        assert!(parse(MIN).unwrap().roles.is_none());
    }

    #[test]
    fn roles_limits_parse_and_unknown_keys_refused() {
        let y = format!(
            "{MIN}roles:\n  id: reg\n  dirs: [a, b/c]\n  limits:\n    total: 3\n    per_role:\n      worker: 2\n      lead: 1\n"
        );
        let r = parse(&y).unwrap().roles.unwrap();
        assert_eq!(r.id, "reg");
        assert_eq!(r.dirs, ["a", "b/c"]);
        assert_eq!(r.limits.total, Some(3));
        assert_eq!(r.limits.per_role["worker"], 2);
        assert_eq!(r.limits.per_role["lead"], 1);
        let e = parse(&format!("{MIN}roles:\n  colour: red\n")).unwrap_err();
        assert!(e.contains("unknown field `colour`"), "{e}");
        let e = parse(&format!("{MIN}roles:\n  limits:\n    most: 3\n")).unwrap_err();
        assert!(e.contains("unknown field `most`"), "{e}");
        let e = parse(&format!("{MIN}roles:\n  limits:\n    total: -1\n")).unwrap_err();
        assert!(
            e.contains("invalid number") || e.contains("invalid value"),
            "{e}"
        );
    }

    #[test]
    fn roles_validate_texts_are_exact() {
        let e = parse(&format!("{MIN}roles:\n  id: ''\n")).unwrap_err();
        assert_eq!(e, "roles.id is empty");
        let e = parse(&format!("{MIN}roles:\n  dirs: []\n")).unwrap_err();
        assert_eq!(
            e,
            "roles.dirs is empty — name at least one folder of role cards"
        );
        let e = parse(&format!("{MIN}roles:\n  dirs: ['a', ' ']\n")).unwrap_err();
        assert_eq!(e, "roles.dirs has an empty folder");
        let e = parse(&format!(
            "{MIN}roles:\n  limits:\n    per_role:\n      '': 1\n"
        ))
        .unwrap_err();
        assert_eq!(e, "roles.limits.per_role has an empty role name");
        let e = parse(&format!(
            "{MIN}connectors:\n  roles:\n    kind: team_git.tools\nroles: {{}}\n"
        ))
        .unwrap_err();
        assert_eq!(e, "agent id `roles` is used twice");
        let e = parse(&format!("{MIN}roles:\n  id: q\n")).unwrap_err();
        assert_eq!(e, "agent id `q` is used twice");
    }

    #[test]
    fn roles_id_tasks_clashes() {
        let e = parse(&format!(
            "{MIN}connectors:\n  tasks:\n    kind: team_git.tools\nroles: {{}}\n"
        ))
        .unwrap_err();
        assert_eq!(e, "agent id `tasks` is used twice");
        // without `roles:` the id is free
        assert!(parse(&format!(
            "{MIN}connectors:\n  tasks:\n    kind: team_git.tools\n"
        ))
        .is_ok());
    }

    #[test]
    fn the_example_enables_roles_with_limits() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/team.yaml");
        let r = load(&path).unwrap().roles.expect("the example has roles");
        assert_eq!(r.id, "roles");
        assert_eq!(r.dirs, ["roles"]);
        assert_eq!(r.limits.total, Some(3));
        assert_eq!(r.limits.per_role["worker"], 2);
    }

    #[test]
    fn tier_fields_carry_the_template_and_no_can_call() {
        let t = tpl(json!({"max_tokens": 4000}));
        assert_eq!(
            tier_fields(&t, "roles_memory"),
            json!({
                "handler_module": "x_backend.tools",
                "model": "m-small",
                "endpoint": "http://127.0.0.1:9/v1",
                "credentials": {"store": "credentials", "ref": "x/127.0.0.1"},
                "price_input_per_mtok": 0.1,
                "price_output_per_mtok": 0.5,
                "memory_store_id": "roles_memory",
                "max_tokens": 4000,
            })
        );
        let v = tier_fields(&t, "m");
        for k in ["can_call", "callable_by", "id", "parent_id", "type"] {
            assert!(v.get(k).is_none(), "{k}");
        }
        let mut bare = tpl(json!({}));
        bare.credentials = None;
        bare.price_input_per_mtok = None;
        bare.price_output_per_mtok = None;
        let v = tier_fields(&bare, "m");
        assert!(v.get("credentials").is_none() && v.get("price_input_per_mtok").is_none());
    }
}
