//! The team's agent views as DATA (#207): one table of declarations, read twice.
//!
//! * The fantastic TUI draws a record's `view.tui` (fantastic#453) with its declared
//!   panel: [`tui_of`] gives that JSON and [`sync_views`] writes it into the records
//!   of the live agents at every start.
//! * The team page draws the same declaration through [`model_of`], a pure function
//!   from the replies of the declared verbs to the page model
//!   `{kind, title, summary: [text], sections: [{name, columns, rows}]}`. The
//!   dispatcher asks the verbs (`dispatcher::panel_reply`) and the page draws the
//!   model as text, as it does for the page-code views of `agent_views`.
//!
//! Every section lists explicit `fields`: the declared panel prints an unlisted
//! reply through `brief`, so a section without fields would show whatever the
//! agent answers. Only read verbs are declared, and the finished page model is
//! scanned for credentials (`agent_views::leaks`) before it is returned.
//!
//! fantastic's panel parser is private to fantastic-tui, so the shape it accepts is
//! mirrored in the tests here (`validate_tui`), not called.

use std::sync::Arc;

use fantastic_kernel::{AgentId, Kernel};
use serde_json::{json, Value};

use crate::agent_views::{leaks, unavailable};

/// Where a section shows its reply (the kinds `view.tui` knows).
pub type Show = &'static str;

/// One section: the reply of one read verb, drawn as `show`.
#[derive(Debug, Clone)]
pub struct Sec {
    /// The section heading.
    pub title: &'static str,
    /// The read verb asked.
    pub verb: &'static str,
    /// Arguments of the call, if the verb takes any.
    pub args: Option<Value>,
    /// `kv`, `table`, `list`, `log` or `text`.
    pub show: Show,
    /// `(dotted path, label)`; never empty.
    pub fields: Vec<(&'static str, &'static str)>,
    /// Refresh period in seconds (the TUI re-asks while the panel is selected).
    pub every_s: Option<u32>,
}

/// The declared views of one kind of agent.
#[derive(Debug, Clone)]
pub struct Decl {
    /// The handler module the declaration belongs to.
    pub handler_module: &'static str,
    /// The service kind the dispatcher lists it under.
    pub kind: &'static str,
    /// The model's title.
    pub title: &'static str,
    /// The sections, top to bottom.
    pub sections: Vec<Sec>,
}

/// The handler module of the queue: a plain store named by the board's `store`.
pub const QUEUE_MODULE: &str = "ram_store.tools";
/// The kinds whose web view is built here (the others keep their page code).
pub const WEB_KINDS: [&str; 6] = ["cadencer", "puller", "webhook", "packs", "pack", "git"];

fn sec(
    title: &'static str,
    verb: &'static str,
    show: Show,
    fields: &[(&'static str, &'static str)],
    every_s: Option<u32>,
) -> Sec {
    Sec {
        title,
        verb,
        args: None,
        show,
        fields: fields.to_vec(),
        every_s,
    }
}

/// Every declaration. One place declares web and TUI.
pub fn decls() -> Vec<Decl> {
    vec![
        Decl {
            handler_module: crate::cadencer::HANDLER_MODULE,
            kind: "cadencer",
            title: "Cadencer",
            sections: vec![
                sec(
                    "Beat",
                    "status",
                    "kv",
                    &[
                        ("settings.enabled", "on"),
                        ("settings.cadence", "cadence"),
                        ("settings.stall_after", "stall_after"),
                        ("ticks", "ticks"),
                        ("last_beat_s", "last_beat_s"),
                        ("next_beat_s", "next_beat_s"),
                        ("digests_sent", "digests_sent"),
                        ("delivery_errors", "delivery_errors"),
                        ("digest_bytes_per_hour", "digest_bytes_per_hour"),
                    ],
                    Some(5),
                ),
                sec(
                    "Load",
                    "status",
                    "kv",
                    &[
                        ("load.workers.stalled", "stalled"),
                        ("load.workers.busy", "busy"),
                        ("load.workers.idle", "idle"),
                        ("load.tasks.queued", "queued"),
                        ("load.tasks.held", "held"),
                        ("load.tasks.in_review", "in_review"),
                        ("load.tasks.blocked", "blocked"),
                        ("load.tasks.done", "done"),
                    ],
                    Some(5),
                ),
            ],
        },
        Decl {
            handler_module: crate::puller::HANDLER_MODULE,
            kind: "puller",
            title: "Puller",
            sections: vec![sec(
                "Pulls",
                "status",
                "kv",
                &[
                    ("repos", "repos"),
                    ("github", "github"),
                    ("store", "store"),
                    ("pulls", "pulls"),
                    ("last_pull_s", "last_pull_s"),
                    ("last_created", "last_created"),
                    ("last_skipped", "last_skipped"),
                    ("total_created", "total_created"),
                ],
                Some(10),
            )],
        },
        Decl {
            handler_module: crate::webhook::HANDLER_MODULE,
            kind: "webhook",
            title: "Webhook",
            sections: vec![
                sec(
                    "Receiver",
                    "status",
                    "kv",
                    &[("enabled", "enabled"), ("secret_set", "secret_set")],
                    Some(10),
                ),
                sec(
                    "Recent events",
                    "status",
                    "table",
                    &[
                        ("event", "event"),
                        ("action", "action"),
                        ("outcome", "outcome"),
                        ("at", "at"),
                    ],
                    Some(10),
                ),
            ],
        },
        Decl {
            handler_module: crate::pack_listing::HANDLER_MODULE,
            kind: "packs",
            title: "Packs",
            sections: vec![sec(
                "Packs",
                "list",
                "table",
                &[
                    ("name", "name"),
                    ("version", "version"),
                    ("installed", "installed"),
                    ("revoked", "revoked"),
                    ("price", "price"),
                ],
                Some(30),
            )],
        },
        Decl {
            handler_module: crate::pack_bundle::HANDLER_MODULE,
            kind: "pack",
            title: "Pack",
            // team_pack has no read verb; its identity is all there is to show.
            sections: vec![sec(
                "Pack",
                "reflect",
                "kv",
                &[("id", "id"), ("sentence", "sentence")],
                None,
            )],
        },
        Decl {
            handler_module: team_github_connector::HANDLER_MODULE,
            kind: "github",
            title: "GitHub",
            // Not listed, on purpose: api_url, directive, git, credentials.
            sections: vec![
                sec(
                    "Recent calls",
                    "reflect",
                    "table",
                    &[("verb", "verb"), ("target", "target"), ("result", "result")],
                    Some(10),
                ),
                sec(
                    "Connector",
                    "reflect",
                    "kv",
                    &[
                        ("repos", "repos"),
                        ("rate.limit", "limit"),
                        ("rate.remaining", "remaining"),
                        ("rate.reset", "reset"),
                    ],
                    Some(10),
                ),
            ],
        },
        Decl {
            handler_module: team_git_connector::HANDLER_MODULE,
            kind: "git",
            title: "Git",
            // Not listed, on purpose: credentials.
            sections: vec![sec(
                "Repository",
                "reflect",
                "kv",
                &[
                    ("remote", "remote"),
                    ("default_branch", "default_branch"),
                    ("repo_dir", "repo_dir"),
                    ("workspace_root", "workspace_root"),
                ],
                None,
            )],
        },
        Decl {
            handler_module: crate::board::HANDLER_MODULE,
            kind: "board",
            title: "Board",
            sections: vec![
                sec(
                    "Agents",
                    "rows",
                    "table",
                    &[
                        ("id", "id"),
                        ("parent", "parent"),
                        ("state", "state"),
                        ("task", "task"),
                        ("idle_s", "idle_s"),
                        ("cost", "cost"),
                    ],
                    Some(5),
                ),
                sec(
                    "Load",
                    "rows",
                    "kv",
                    &[
                        ("load.workers.busy", "busy"),
                        ("load.workers.idle", "idle"),
                        ("load.workers.stalled", "stalled"),
                        ("load.tasks.queued", "queued"),
                        ("load.tasks.held", "held"),
                        ("load.tasks.in_review", "in_review"),
                        ("load.tasks.blocked", "blocked"),
                        ("load.tasks.done", "done"),
                        ("load.done_last_hour", "done_last_hour"),
                    ],
                    Some(5),
                ),
            ],
        },
        Decl {
            handler_module: crate::dispatcher::HANDLER_MODULE,
            kind: "dispatcher",
            title: "Dispatcher",
            // `tree` answers {kind, id, children}: groups and loose workers, a worker
            // as {kind, row: {id, state, task, minutes, cost}}.
            sections: vec![
                sec(
                    "Fleet",
                    "sidebar",
                    "kv",
                    &[("fleet.running", "running"), ("fleet.max", "max")],
                    Some(5),
                ),
                sec(
                    "Team",
                    "tree",
                    "table",
                    &[
                        ("kind", "kind"),
                        ("id", "group"),
                        ("row.id", "worker"),
                        ("row.state", "state"),
                        ("row.task", "task"),
                        ("row.minutes", "minutes"),
                        ("row.cost", "cost"),
                    ],
                    Some(5),
                ),
            ],
        },
        Decl {
            handler_module: QUEUE_MODULE,
            kind: "queue",
            title: "Queue",
            // Counts only: the store's `read` answers whole rows (the task text).
            sections: vec![sec(
                "Store",
                "reflect",
                "kv",
                &[
                    ("id", "id"),
                    ("durable", "durable"),
                    ("owners", "owners"),
                    ("rows", "rows"),
                ],
                Some(10),
            )],
        },
    ]
}

/// The declaration of a handler module (the queue's store module included; see
/// [`declared_agents`] for which `ram_store` agent is the queue).
pub fn decl_for(handler_module: &str) -> Option<Decl> {
    decls()
        .into_iter()
        .find(|d| d.handler_module == handler_module)
}

/// The declaration of a service kind.
pub fn decl_by_kind(kind: &str) -> Option<Decl> {
    decls().into_iter().find(|d| d.kind == kind)
}

/// The `view.tui` JSON of a declaration: `[{title, call: {verb, args?}, show,
/// fields: [{path, label}], every_s?}]`.
pub fn tui_of(d: &Decl) -> Value {
    Value::Array(
        d.sections
            .iter()
            .map(|s| {
                let mut call = json!({"verb": s.verb});
                if let Some(a) = &s.args {
                    call["args"] = a.clone();
                }
                let fields: Vec<Value> = s
                    .fields
                    .iter()
                    .map(|(path, label)| {
                        if path == label {
                            json!(path)
                        } else {
                            json!({"path": path, "label": label})
                        }
                    })
                    .collect();
                let mut o = json!({
                    "title": s.title,
                    "call": call,
                    "show": s.show,
                    "fields": fields,
                });
                if let Some(n) = s.every_s {
                    o["every_s"] = json!(n);
                }
                o
            })
            .collect(),
    )
}

/// The value at a dotted path; a numeric segment indexes a list (fantastic's rule).
pub fn pick<'a>(v: &'a Value, dotted: &str) -> Option<&'a Value> {
    let mut cur = v;
    for seg in dotted.split('.') {
        cur = match cur {
            Value::Array(a) => a.get(seg.parse::<usize>().ok()?)?,
            Value::Object(o) => o.get(seg)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// A value as cell text: a string as it is, a number or a bool printed, a list of
/// strings joined, anything else `-`.
fn cell(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Array(a)) if !a.is_empty() && a.iter().all(Value::is_string) => a
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        _ => "-".into(),
    }
}

/// The most rows a table shows (fantastic's panel draws 200 lines at most).
const MAX_ROWS: usize = 200;

/// The rows of a table, list or log: the reply when it is a list, else its first
/// list-valued key in sorted-key order.
fn items(reply: &Value) -> Vec<&Value> {
    match reply {
        Value::Array(a) => a.iter().collect(),
        Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            keys.into_iter()
                .find_map(|k| o[k].as_array())
                .map(|a| a.iter().collect())
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

/// One section of the page model, with its one summary line (if any).
fn section_of(s: &Sec, reply: &Value) -> (Value, Option<String>) {
    let labels: Vec<&str> = s.fields.iter().map(|(_, l)| *l).collect();
    match s.show {
        "kv" => {
            let rows: Vec<Vec<String>> = s
                .fields
                .iter()
                .map(|(p, l)| vec![l.to_string(), cell(pick(reply, p))])
                .collect();
            let line = rows.first().map(|r| format!("{} {}", r[0], r[1]));
            (
                json!({"name": s.title, "columns": ["field", "value"], "rows": rows}),
                line,
            )
        }
        "table" => {
            let rows: Vec<Vec<String>> = items(reply)
                .into_iter()
                .take(MAX_ROWS)
                .map(|r| s.fields.iter().map(|(p, _)| cell(pick(r, p))).collect())
                .collect();
            let line = Some(format!("{} {}", rows.len(), s.title.to_lowercase()));
            (
                json!({"name": s.title, "columns": labels, "rows": rows}),
                line,
            )
        }
        "list" | "log" => {
            let path = s.fields.first().map(|(p, _)| *p).unwrap_or("");
            let rows: Vec<Vec<String>> = items(reply)
                .into_iter()
                .take(MAX_ROWS)
                .map(|r| vec![cell(pick(r, path).or(Some(r)))])
                .collect();
            let line = Some(format!("{} {}", rows.len(), s.title.to_lowercase()));
            let col = labels.first().copied().unwrap_or("value");
            (
                json!({"name": s.title, "columns": [col], "rows": rows}),
                line,
            )
        }
        _ => {
            let path = s.fields.first().map(|(p, _)| *p).unwrap_or("");
            (
                json!({"name": s.title, "columns": [s.title], "rows": [[cell(pick(reply, path))]]}),
                None,
            )
        }
    }
}

/// The page model of a declaration, from the replies of its sections' verbs (one
/// reply per section, in order). Pure. A reply that is an error, is not an object
/// or is missing gives the "unavailable" model, never the raw error; so does a
/// finished model that looks like it holds a credential.
pub fn model_of(d: &Decl, replies: &[Value]) -> Value {
    let down = || unavailable(d.kind, d.title);
    let mut sections = Vec::new();
    let mut summary = Vec::new();
    for (i, s) in d.sections.iter().enumerate() {
        let Some(reply) = replies.get(i) else {
            return down();
        };
        if !reply.is_object() || reply.get("error").is_some() {
            return down();
        }
        let (section, line) = section_of(s, reply);
        sections.push(section);
        summary.extend(line);
    }
    let model = json!({
        "kind": d.kind,
        "title": d.title,
        "summary": summary,
        "sections": sections,
    });
    if leaks(&model.to_string()) {
        return down();
    }
    model
}

/// Every live agent that has a declaration: by handler module, plus the queue (the
/// store named by a board record's `store`; another `ram_store` agent has none).
/// Sorted by id.
pub fn declared_agents(kernel: &Kernel) -> Vec<(AgentId, Decl)> {
    let mut out: Vec<(AgentId, Decl)> = Vec::new();
    let mut stores: Vec<String> = Vec::new();
    for e in kernel.agents.iter() {
        let a = e.value();
        let Some(m) = a.handler_module.as_deref() else {
            continue;
        };
        if m == crate::board::HANDLER_MODULE {
            if let Some(s) = a
                .meta
                .read()
                .ok()
                .and_then(|m| m.get("store").and_then(Value::as_str).map(str::to_string))
            {
                stores.push(s);
            }
        }
        if m == QUEUE_MODULE {
            continue;
        }
        if let Some(d) = decl_for(m) {
            out.push((a.id.clone(), d));
        }
    }
    if let Some(q) = decl_for(QUEUE_MODULE) {
        for s in stores {
            let id = AgentId::from(s.as_str());
            let is_ram = kernel
                .agents
                .get(&id)
                .is_some_and(|a| a.handler_module.as_deref() == Some(QUEUE_MODULE));
            if is_ram && !out.iter().any(|(i, _)| *i == id) {
                out.push((id, q.clone()));
            }
        }
    }
    out.sort_by(|a, b| a.0 .0.cmp(&b.0 .0));
    out
}

/// Write `view.tui` into the record of every declared live agent, keeping the rest of
/// its `view` (kind, bytes, calls). Idempotent: a record that already holds the
/// declaration is not touched. Returns how many records changed.
pub fn sync_views(kernel: &Arc<Kernel>) -> usize {
    let mut changed = 0;
    for (id, d) in declared_agents(kernel) {
        let tui = tui_of(&d);
        let mut view = kernel
            .agents
            .get(&id)
            .and_then(|a| a.meta.read().ok().and_then(|m| m.get("view").cloned()))
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        if view.get("tui") == Some(&tui) {
            continue;
        }
        view.insert("tui".into(), tui);
        let mut patch = serde_json::Map::new();
        patch.insert("view".into(), Value::Object(view));
        if kernel.update(&id, patch).is_some() {
            changed += 1;
        } else {
            eprintln!("declared views: `{id}` went away before its view was written");
        }
    }
    changed
}

/// The verbs a declaration reads, once each, in order.
pub fn read_verbs(d: &Decl) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    for s in &d.sections {
        if !v.contains(&s.verb) {
            v.push(s.verb);
        }
    }
    v
}

/// `existing` with `want` added, or `None` when it already covers them (a `*` or every
/// verb present). Keeps the verbs another grant put there.
fn merged(existing: Option<&Value>, want: &[&str]) -> Option<Value> {
    match existing {
        Some(Value::String(s)) if s == "*" => None,
        Some(Value::Array(a)) => {
            let mut out = a.clone();
            for w in want {
                if !out.iter().any(|v| v.as_str() == Some(w)) {
                    out.push(json!(w));
                }
            }
            if out.len() == a.len() {
                None
            } else {
                Some(Value::Array(out))
            }
        }
        _ => Some(json!(want)),
    }
}

/// Grant the dispatcher the read verbs each declared agent's
/// declaration names, on both ends (the agent's `callable_by`, the caller's
/// `can_call`), never `*`. Verbs another grant put there stay. Idempotent. Returns
/// how many grants were written.
pub fn grant_views(kernel: &Arc<Kernel>) -> usize {
    let mut written = 0;
    for principal in [crate::tree::DISPATCHER_ID] {
        let who = AgentId::from(principal);
        if !kernel.agents.contains_key(&who) {
            continue;
        }
        for (id, d) in declared_agents(kernel) {
            if id == who {
                continue;
            }
            let want = read_verbs(&d);
            let by_agent =
                kernel.edit_callable_by(&id, |cb| match merged(cb.get(principal), &want) {
                    Some(v) => {
                        cb.insert(principal.into(), v);
                        true
                    }
                    None => false,
                });
            let by_caller =
                kernel.edit_can_call(&who, |cc| match merged(cc.get(id.as_str()), &want) {
                    Some(v) => {
                        cc.insert(id.0.clone(), v);
                        true
                    }
                    None => false,
                });
            match (by_agent, by_caller) {
                (Ok(a), Ok(b)) => written += usize::from(a) + usize::from(b),
                (Err(e), _) | (_, Err(e)) => {
                    eprintln!("declared views: `{principal}` -> `{id}`: {e}")
                }
            }
        }
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;
    use fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
    use fantastic_kernel::Item;

    /// A local validator that mirrors fantastic's `view.tui` parser (fantastic-tui
    /// `panel::declared::parse_tui`, private there).
    fn validate_tui(tui: &Value) -> Result<(), String> {
        let list = tui.as_array().ok_or("not a list")?;
        if list.is_empty() || list.len() > 16 {
            return Err(format!("{} sections", list.len()));
        }
        for (i, s) in list.iter().enumerate() {
            let o = s.as_object().ok_or(format!("[{i}] not an object"))?;
            for k in o.keys() {
                if !["title", "call", "show", "fields", "every_s"].contains(&k.as_str()) {
                    return Err(format!("[{i}] unknown key {k}"));
                }
            }
            if o.get("title")
                .and_then(Value::as_str)
                .is_none_or(|t| t.trim().is_empty())
            {
                return Err(format!("[{i}] title"));
            }
            let call = o
                .get("call")
                .and_then(Value::as_object)
                .ok_or(format!("[{i}] call"))?;
            if call.keys().any(|k| k != "verb" && k != "args") {
                return Err(format!("[{i}] call key"));
            }
            if call
                .get("verb")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            {
                return Err(format!("[{i}] verb"));
            }
            match call.get("args") {
                None | Some(Value::Null) | Some(Value::Object(_)) => {}
                Some(_) => return Err(format!("[{i}] args")),
            }
            match o.get("show").and_then(Value::as_str) {
                Some("kv" | "table" | "list" | "log" | "text") => {}
                other => return Err(format!("[{i}] show {other:?}")),
            }
            match o.get("fields") {
                None | Some(Value::Null) => {}
                Some(Value::Array(a)) => {
                    for f in a {
                        match f {
                            Value::String(p) if !p.is_empty() => {}
                            Value::Object(fo) => {
                                if fo.keys().any(|k| k != "path" && k != "label") {
                                    return Err(format!("[{i}] field key"));
                                }
                                if fo
                                    .get("path")
                                    .and_then(Value::as_str)
                                    .is_none_or(str::is_empty)
                                {
                                    return Err(format!("[{i}] field path"));
                                }
                                if fo.get("label").is_some_and(|l| !l.is_string()) {
                                    return Err(format!("[{i}] field label"));
                                }
                            }
                            _ => return Err(format!("[{i}] field")),
                        }
                    }
                }
                Some(_) => return Err(format!("[{i}] fields")),
            }
            match o.get("every_s") {
                None | Some(Value::Null) => {}
                Some(v) => {
                    if !v.as_f64().is_some_and(|n| (1.0..=3600.0).contains(&n)) {
                        return Err(format!("[{i}] every_s"));
                    }
                }
            }
        }
        Ok(())
    }

    fn decl(module: &str) -> Decl {
        decl_for(module).unwrap_or_else(|| panic!("no declaration for {module}"))
    }

    #[test]
    fn every_declaration_parses_as_a_valid_view_tui() {
        for d in decls() {
            let tui = tui_of(&d);
            validate_tui(&tui).unwrap_or_else(|e| panic!("{}: {e}: {tui}", d.kind));
        }
        // The validator is real: a mutated declaration is refused.
        let mut d = decl(crate::cadencer::HANDLER_MODULE);
        d.sections[0].show = "grid";
        assert!(validate_tui(&tui_of(&d)).is_err());
        let mut d = decl(crate::cadencer::HANDLER_MODULE);
        d.sections[0].every_s = Some(0);
        assert!(validate_tui(&tui_of(&d)).is_err());
        let mut d = decl(crate::cadencer::HANDLER_MODULE);
        d.sections[0].verb = "";
        assert!(validate_tui(&tui_of(&d)).is_err());
        assert!(validate_tui(&json!([])).is_err());
        assert!(validate_tui(
            &json!([{"title": "t", "call": {"verb": "v"}, "show": "kv", "x": 1}])
        )
        .is_err());
    }

    #[test]
    fn tui_of_shapes_the_call_and_the_fields() {
        let tui = tui_of(&decl(crate::cadencer::HANDLER_MODULE));
        assert_eq!(tui[0]["call"], json!({"verb": "status"}));
        assert_eq!(tui[0]["every_s"], 5);
        assert_eq!(
            tui[0]["fields"][0],
            json!({"path": "settings.enabled", "label": "on"})
        );
        assert_eq!(tui[0]["fields"][3], json!("ticks"));
        // No every_s when none is declared.
        assert!(tui_of(&decl(team_git_connector::HANDLER_MODULE))[0]
            .get("every_s")
            .is_none());
        // Arguments ride on the call.
        let mut d = decl(crate::pack_listing::HANDLER_MODULE);
        d.sections[0].args = Some(json!({"limit": 5}));
        assert_eq!(tui_of(&d)[0]["call"]["args"], json!({"limit": 5}));
    }

    #[test]
    fn declarations_cover_every_team_kind() {
        let all = decls();
        let kinds: Vec<&str> = all.iter().map(|d| d.kind).collect();
        for k in [
            "cadencer",
            "puller",
            "webhook",
            "packs",
            "pack",
            "github",
            "git",
            "board",
            "dispatcher",
            "queue",
        ] {
            assert!(kinds.contains(&k), "no declaration for {k}");
        }
        for d in &all {
            assert!(!d.sections.is_empty(), "{}", d.kind);
            for s in &d.sections {
                assert!(
                    !s.fields.is_empty(),
                    "{} / {} has no fields",
                    d.kind,
                    s.title
                );
            }
        }
        for k in WEB_KINDS {
            assert!(decl_by_kind(k).is_some(), "{k}");
        }
    }

    #[test]
    fn declared_verbs_are_read_only() {
        let writes = [
            "pull",
            "receive",
            "set",
            "configure",
            "tick",
            "install",
            "pack",
            "eval",
            "publish",
            "submit",
            "fetch",
            "report",
            "takedown",
            "control",
            "open",
            "commit",
            "push",
            "task",
            "agent",
            "ask",
            "audit",
        ];
        let reg = crate::registry();
        for d in decls() {
            let docs = reg
                .get(d.handler_module)
                .unwrap_or_else(|| panic!("{} is not registered", d.handler_module))
                .verb_docs();
            for s in &d.sections {
                assert!(
                    !writes.contains(&s.verb) && !s.verb.starts_with("worktree_") && s.verb != "*",
                    "{}: {} writes",
                    d.kind,
                    s.verb
                );
                assert!(
                    s.verb == "reflect" || docs.get(s.verb).is_some(),
                    "{}: `{}` is not a verb of {}",
                    d.kind,
                    s.verb,
                    d.handler_module
                );
            }
        }
    }

    fn cadencer_status() -> Value {
        // The shape of the cadencer's real `status` reply (cadencer.rs, "status").
        json!({
            "settings": {"enabled": true, "cadence": 300, "stall_after": 1800, "quiet": false, "off": []},
            "ticks": 7, "last_beat_s": 1760000100u64, "next_beat_s": 1760000400u64,
            "digests_sent": 3, "delivery_errors": 0,
            "digest_bytes_per_hour": 1200, "driver_tokens_per_hour": 55,
            "load": {
                "workers": {"busy": 2, "idle": 1, "stalled": 4},
                "groups": {"g": {"busy": 2, "idle": 1, "stalled": 4, "driver": "d"}},
                "tasks": {"held": 2, "in_review": 1, "done": 9, "blocked": 0, "queued": 5},
                "done_last_hour": 2, "tokens_total": 10,
                "digest_bytes_per_hour": 1200, "driver_tokens_per_hour": 55}
        })
    }

    fn rows_of_section<'a>(m: &'a Value, name: &str) -> &'a Vec<Value> {
        m["sections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == name)
            .unwrap_or_else(|| panic!("no section {name}: {m}"))["rows"]
            .as_array()
            .unwrap()
    }

    fn value_of(rows: &[Value], label: &str) -> Value {
        rows.iter()
            .find(|r| r[0] == label)
            .unwrap_or_else(|| panic!("no row {label}"))[1]
            .clone()
    }

    #[test]
    fn cadencer_view_renders_a_status_reply() {
        let d = decl(crate::cadencer::HANDLER_MODULE);
        let s = cadencer_status();
        let m = model_of(&d, &[s.clone(), s]);
        assert_eq!(m["kind"], "cadencer");
        let beat = rows_of_section(&m, "Beat");
        assert_eq!(value_of(beat, "on"), "true");
        assert_eq!(value_of(beat, "last_beat_s"), "1760000100");
        assert_eq!(value_of(beat, "next_beat_s"), "1760000400");
        assert_eq!(value_of(beat, "digests_sent"), "3");
        let load = rows_of_section(&m, "Load");
        assert_eq!(value_of(load, "stalled"), "4");
        let summary = m["summary"].to_string();
        assert!(summary.contains("on true"), "{summary}");
        assert!(summary.contains("stalled 4"), "{summary}");
    }

    #[test]
    fn the_cadencers_real_status_reply_renders() {
        // The reply a live cadencer gives, not a hand-written fixture.
        let dir = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let status = rt.block_on(async {
            let kernel = crate::start(dir.path()).await.unwrap();
            kernel
                .send(vec![Item::new(
                    DEFAULT_ROOT_ID,
                    json!({"type": "create_agent", "id": "cad", "parent_id": DEFAULT_ROOT_ID,
                        "handler_module": crate::cadencer::HANDLER_MODULE}),
                )])
                .await;
            let out = kernel
                .send(vec![Item::new("cad", json!({"type": "status"}))])
                .await;
            out[0].result.clone()
        });
        let m = model_of(
            &decl(crate::cadencer::HANDLER_MODULE),
            &[status.clone(), status],
        );
        assert_ne!(m["summary"], json!(["unavailable"]), "{m}");
        let beat = rows_of_section(&m, "Beat");
        assert_ne!(value_of(beat, "on"), "-");
        assert_ne!(value_of(beat, "ticks"), "-");
        assert_ne!(value_of(beat, "digests_sent"), "-");
    }

    #[test]
    fn packs_view_renders_a_list_reply() {
        let reply = json!({"packs": [
            {"name": "alpha", "version": "0.1", "summary": "SUMMARYTEXT", "price": 5, "installed": true, "revoked": false},
            {"name": "mid", "version": "2.0", "summary": "Mid", "price": 1, "installed": false, "revoked": true}
        ]});
        let m = model_of(&decl(crate::pack_listing::HANDLER_MODULE), &[reply]);
        let rows = rows_of_section(&m, "Packs");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], json!(["alpha", "0.1", "true", "false", "5"]));
        assert_eq!(rows[1][3], "true");
        assert!(!m.to_string().contains("SUMMARYTEXT"));
    }

    #[test]
    fn pack_view_declares_reflect_only() {
        let d = decl(crate::pack_bundle::HANDLER_MODULE);
        assert_eq!(d.sections.len(), 1);
        assert_eq!(d.sections[0].verb, "reflect");
        let m = model_of(
            &d,
            &[json!({"id": "pack", "sentence": "Packs a worker.", "extra": "x"})],
        );
        let rows = rows_of_section(&m, "Pack");
        assert_eq!(value_of(rows, "id"), "pack");
        assert!(!m.to_string().contains("\"extra\"") && !m.to_string().contains(":\"x\""));
    }

    fn secret_reflect() -> Value {
        json!({
            "id": "github", "sentence": "GitHub.",
            "repos": ["acme/widgets"],
            "api_url": format!("https://{}:{}@host", "x-access-token", "ghs_SECRET"),
            "directive": "use Bearer abc",
            "credentials": {"store": "credentials", "ref": "ghp_SECRET"},
            "git": "git",
            "extra": "eyJunlisted",
            "remote": "origin", "default_branch": "main", "repo_dir": "/r", "workspace_root": "/w",
            "recent_calls": [{"verb": "issues", "target": "acme/widgets", "result": "ok"}],
            "rate": {"limit": 5000, "remaining": 4990, "reset": 1760000000u64}
        })
    }

    #[test]
    fn github_and_git_views_never_show_credentials() {
        let r = secret_reflect();
        let bad = [
            "ghs_SECRET",
            "ghp_SECRET",
            "Bearer abc",
            "x-access-token",
            "eyJunlisted",
            "credentials",
        ];
        for module in [
            team_github_connector::HANDLER_MODULE,
            team_git_connector::HANDLER_MODULE,
        ] {
            let d = decl(module);
            for s in &d.sections {
                for (path, _) in &s.fields {
                    if let Some(v) = pick(&r, path) {
                        let t = v.to_string();
                        for b in bad {
                            assert!(!t.contains(b), "{module}: path {path} shows {b}");
                        }
                    }
                }
            }
            let replies = vec![r.clone(); d.sections.len()];
            let m = model_of(&d, &replies);
            assert_ne!(m["summary"], json!(["unavailable"]), "{module}: {m}");
            let t = m.to_string();
            for b in bad {
                assert!(!t.contains(b), "{module}: model shows {b}");
            }
        }
        // The git view shows its listed fields.
        let g = model_of(&decl(team_git_connector::HANDLER_MODULE), &[r]);
        assert_eq!(
            value_of(rows_of_section(&g, "Repository"), "remote"),
            "origin"
        );
    }

    /// A reply for `d` with `plant` under every key a field does not name, and
    /// (`in_fields`) under every listed key too.
    fn planted_reply(d: &Decl, s: &Sec, plant: &str, in_fields: bool) -> Value {
        let mut reply = json!({});
        // Unlisted keys at the top and inside a row.
        for k in [
            "api_url",
            "directive",
            "credentials",
            "token",
            "authorization",
            "summary",
            "url",
        ] {
            reply[k] = json!(plant);
        }
        reply["rows"] = json!([{"unlisted": plant}]);
        reply["packs"] = json!([{"summary": plant, "url": plant}]);
        reply["events"] = json!([{"unlisted": plant}]);
        reply["recent_calls"] = json!([{"unlisted": plant}]);
        reply["children"] = json!([{"unlisted": plant}]);
        if in_fields {
            for (path, _) in &s.fields {
                // Build the nested object for the dotted path (a table field sits in a row).
                let mut leaf = json!(plant);
                for seg in path.split('.').rev() {
                    leaf = json!({ seg: leaf });
                }
                let row = leaf.clone();
                if let (Some(dst), Some(src)) = (reply.as_object_mut(), leaf.as_object()) {
                    for (k, v) in src {
                        dst.insert(k.clone(), v.clone());
                    }
                }
                for list in ["rows", "packs", "events", "recent_calls", "children"] {
                    reply[list] = json!([row.clone()]);
                }
            }
        }
        let _ = d;
        reply
    }

    #[test]
    fn no_token_in_any_rendered_view() {
        let tokens = [
            "ghs_TOKEN123",
            "ghp_TOKEN123",
            "Bearer TOKEN123",
            "eyJTOKEN123",
        ];
        for d in decls() {
            for t in tokens {
                // Pass one: the token is only in keys no field names.
                let replies: Vec<Value> = d
                    .sections
                    .iter()
                    .map(|s| planted_reply(&d, s, t, false))
                    .collect();
                let m = model_of(&d, &replies);
                assert!(!m.to_string().contains(t), "{}: {t} leaked: {m}", d.kind);
                // Pass two: the token is in a listed field. The model is "unavailable".
                let replies: Vec<Value> = d
                    .sections
                    .iter()
                    .map(|s| planted_reply(&d, s, t, true))
                    .collect();
                let m = model_of(&d, &replies);
                assert!(!m.to_string().contains(t), "{}: {t} leaked: {m}", d.kind);
                assert_eq!(m["summary"], json!(["unavailable"]), "{}: {t}: {m}", d.kind);
            }
        }
    }

    #[test]
    fn a_refusing_service_gives_an_unavailable_model() {
        let d = decl(crate::cadencer::HANDLER_MODULE);
        let refusal = json!({"error": "RAWERRORTEXT", "reason": "RAWREASON"});
        let m = model_of(&d, &[refusal.clone(), refusal]);
        assert_eq!(m["summary"], json!(["unavailable"]));
        assert_eq!(m["kind"], "cadencer");
        let t = m.to_string();
        assert!(
            !t.contains("RAWERRORTEXT") && !t.contains("RAWREASON"),
            "{t}"
        );
        // A non-object reply and a missing reply are the same.
        assert_eq!(
            model_of(&d, &[json!("x"), json!(1)])["summary"],
            json!(["unavailable"])
        );
        assert_eq!(
            model_of(&d, &[cadencer_status()])["summary"],
            json!(["unavailable"])
        );
    }

    #[test]
    fn model_of_table_takes_first_list_key_and_kv_picks_dotted_paths() {
        let r = json!({"zeta": [{"a": 1}], "alpha": [{"a": 2, "b": true}], "load": {"1m": 0.5}});
        assert_eq!(pick(&r, "load.1m"), Some(&json!(0.5)));
        assert_eq!(pick(&r, "alpha.0.a"), Some(&json!(2)));
        assert_eq!(pick(&r, "alpha.9.a"), None);
        assert_eq!(pick(&r, "alpha.x"), None);
        let d = Decl {
            handler_module: "x",
            kind: "x",
            title: "X",
            sections: vec![
                sec(
                    "T",
                    "v",
                    "table",
                    &[("a", "a"), ("b", "b"), ("missing", "m")],
                    None,
                ),
                sec(
                    "K",
                    "v",
                    "kv",
                    &[
                        ("load.1m", "load"),
                        ("alpha.0.a", "first"),
                        ("nope", "nope"),
                        ("alpha.0", "obj"),
                    ],
                    None,
                ),
            ],
        };
        let m = model_of(&d, &[r.clone(), r]);
        // The first list key in sorted order is `alpha`.
        assert_eq!(rows_of_section(&m, "T"), &vec![json!(["2", "true", "-"])]);
        let kv = rows_of_section(&m, "K");
        assert_eq!(value_of(kv, "load"), "0.5");
        assert_eq!(value_of(kv, "first"), "2");
        assert_eq!(value_of(kv, "nope"), "-");
        assert_eq!(value_of(kv, "obj"), "-");
        // A top-level list is the rows.
        let list = json!([{"a": 1}, {"a": 2}]);
        assert_eq!(items(&list).len(), 2);
    }

    // ---- records: sync_views, grant_views ----

    async fn mk(kernel: &Arc<Kernel>, rec: Value) {
        let mut rec = rec;
        rec["type"] = json!("create_agent");
        rec["parent_id"] = json!(DEFAULT_ROOT_ID);
        let out = kernel.send(vec![Item::new(DEFAULT_ROOT_ID, rec)]).await;
        assert!(out[0].result.get("error").is_none(), "{}", out[0].result);
    }

    fn view_of(kernel: &Arc<Kernel>, id: &str) -> Value {
        kernel
            .agents
            .get(&AgentId::from(id))
            .map(|a| {
                a.meta
                    .read()
                    .unwrap()
                    .get("view")
                    .cloned()
                    .unwrap_or(Value::Null)
            })
            .unwrap_or(Value::Null)
    }

    fn field_of(kernel: &Arc<Kernel>, id: &str, f: &str) -> Value {
        kernel
            .agents
            .get(&AgentId::from(id))
            .map(|a| {
                a.meta
                    .read()
                    .unwrap()
                    .get(f)
                    .cloned()
                    .unwrap_or(Value::Null)
            })
            .unwrap_or(Value::Null)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sync_views_puts_view_tui_on_declared_agents() {
        let dir = tempfile::tempdir().unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        mk(
            &kernel,
            json!({"id": "cad", "handler_module": crate::cadencer::HANDLER_MODULE,
                "view": {"kind": "html", "calls": ["x"]}}),
        )
        .await;
        mk(
            &kernel,
            json!({"id": "pul", "handler_module": crate::puller::HANDLER_MODULE}),
        )
        .await;
        let n = sync_views(&kernel);
        assert_eq!(n, 2);
        let cad = view_of(&kernel, "cad");
        assert_eq!(cad["tui"], tui_of(&decl(crate::cadencer::HANDLER_MODULE)));
        assert_eq!(cad["kind"], "html", "kind survives: {cad}");
        assert_eq!(cad["calls"], json!(["x"]), "calls survive: {cad}");
        let pul = view_of(&kernel, "pul");
        assert_eq!(pul["tui"], tui_of(&decl(crate::puller::HANDLER_MODULE)));
        assert!(
            pul.get("kind").is_none(),
            "a tui-only view has no kind: {pul}"
        );
        // Idempotent: nothing changes the second time.
        assert_eq!(sync_views(&kernel), 0);
        assert_eq!(view_of(&kernel, "cad"), cad);
        // A stale declaration is replaced.
        let mut stale = cad.clone();
        stale["tui"] = json!([{"title": "Old", "call": {"verb": "status"}, "show": "kv"}]);
        let mut patch = serde_json::Map::new();
        patch.insert("view".into(), stale);
        kernel.update(&AgentId::from("cad"), patch);
        assert_eq!(sync_views(&kernel), 1);
        assert_eq!(view_of(&kernel, "cad"), cad);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sync_views_declares_the_queue_and_leaves_other_agents_alone() {
        let dir = tempfile::tempdir().unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        mk(
            &kernel,
            json!({"id": "queue", "handler_module": QUEUE_MODULE}),
        )
        .await;
        mk(
            &kernel,
            json!({"id": "memory", "handler_module": QUEUE_MODULE}),
        )
        .await;
        // Before a board names it, no ram_store is the queue.
        assert_eq!(sync_views(&kernel), 0);
        assert!(view_of(&kernel, "queue").is_null());
        mk(
            &kernel,
            json!({"id": "bd", "handler_module": crate::board::HANDLER_MODULE, "store": "queue"}),
        )
        .await;
        sync_views(&kernel);
        assert_eq!(
            view_of(&kernel, "queue")["tui"],
            tui_of(&decl(QUEUE_MODULE))
        );
        assert!(
            view_of(&kernel, "memory").is_null(),
            "another ram_store gets no view: {}",
            view_of(&kernel, "memory")
        );
        // The board itself is declared too.
        assert!(view_of(&kernel, "bd")["tui"].is_array());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn grant_views_names_read_verbs_not_star() {
        let dir = tempfile::tempdir().unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        mk(
            &kernel,
            json!({"id": "dispatcher", "handler_module": crate::dispatcher::HANDLER_MODULE}),
        )
        .await;
        mk(
            &kernel,
            json!({"id": "cad", "handler_module": crate::cadencer::HANDLER_MODULE}),
        )
        .await;
        mk(
            &kernel,
            json!({"id": "web", "handler_module": crate::webhook::HANDLER_MODULE}),
        )
        .await;
        mk(
            &kernel,
            json!({"id": "gh", "handler_module": team_github_connector::HANDLER_MODULE,
                "app": {"credentials": {"store": "credentials", "ref": "github/app"}},
                "repos": ["o/r"], "directive": "Push a work branch.",
                "callable_by": {"dispatcher": "*"}}),
        )
        .await;
        assert!(grant_views(&kernel) > 0);
        let cb = field_of(&kernel, "web", "callable_by");
        assert_eq!(cb["dispatcher"], json!(["status"]), "{cb}");
        assert_ne!(cb["dispatcher"], json!("*"));
        let cc = field_of(&kernel, "dispatcher", "can_call");
        assert_eq!(cc["web"], json!(["status"]), "{cc}");
        let cad = field_of(&kernel, "cad", "callable_by");
        assert_eq!(cad["dispatcher"], json!(["status"]), "{cad}");
        // A star the operator wrote is never narrowed.
        assert_eq!(
            field_of(&kernel, "gh", "callable_by")["dispatcher"],
            json!("*")
        );
        // Idempotent.
        assert_eq!(grant_views(&kernel), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn grant_views_does_not_grant_the_removed_view() {
        let dir = tempfile::tempdir().unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        mk(
            &kernel,
            json!({"id": "dispatcher_view", "handler_module": "ram_store.tools"}),
        )
        .await;
        mk(
            &kernel,
            json!({"id": "dispatcher", "handler_module": crate::dispatcher::HANDLER_MODULE}),
        )
        .await;
        mk(
            &kernel,
            json!({"id": "web", "handler_module": crate::webhook::HANDLER_MODULE}),
        )
        .await;
        assert!(grant_views(&kernel) > 0);
        for entry in kernel.agents.iter() {
            let meta = entry.meta.read().unwrap().clone();
            assert!(
                meta["callable_by"].get("dispatcher_view").is_none(),
                "{}: {meta:?}",
                entry.id.0
            );
        }
        assert!(field_of(&kernel, "dispatcher_view", "can_call")
            .get("web")
            .is_none());
    }
}
