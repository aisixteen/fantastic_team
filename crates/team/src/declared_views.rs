//! The team's agent views as DATA (#207): one table of declarations.
//!
//! The team page draws a declaration through [`model_of`], a pure function
//! from the replies of the declared verbs to the page model
//! `{kind, title, summary: [text], sections: [{name, columns, rows}]}`. The
//! dispatcher asks the verbs (`dispatcher::panel_reply`) and the page draws the
//! model as text, as it does for the page-code views of `agent_views`.
//!
//! Every section lists explicit `fields`. Only read verbs are declared.

use std::sync::Arc;

use fantastic_kernel::{AgentId, Kernel};
use serde_json::{json, Value};

use crate::agent_views::unavailable;

/// Where a section shows its reply.
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

/// The kinds whose web view is built here (the others keep their page code).
pub const WEB_KINDS: [&str; 2] = ["cadencer", "git"];

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

/// Every declaration.
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
                        ("auth.mode", "auth"),
                        ("auth.note", "note"),
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
    ]
}

/// The declaration of a handler module.
pub fn decl_for(handler_module: &str) -> Option<Decl> {
    decls()
        .into_iter()
        .find(|d| d.handler_module == handler_module)
}

/// The declaration of a service kind.
pub fn decl_by_kind(kind: &str) -> Option<Decl> {
    decls().into_iter().find(|d| d.kind == kind)
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
    model
}

/// Every live agent that has a declaration, by handler module. Sorted by id.
pub fn declared_agents(kernel: &Kernel) -> Vec<(AgentId, Decl)> {
    let mut out: Vec<(AgentId, Decl)> = Vec::new();
    for e in kernel.agents.iter() {
        let a = e.value();
        let Some(m) = a.handler_module.as_deref() else {
            continue;
        };
        if let Some(d) = decl_for(m) {
            out.push((a.id.clone(), d));
        }
    }
    out.sort_by(|a, b| a.0 .0.cmp(&b.0 .0));
    out
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

/// The html views the team page draws (each agent's `reflect` carries its view): the
/// handler module and the verbs that view calls on its agent.
pub fn page_views() -> [(&'static str, &'static [&'static str]); 4] {
    [
        (
            crate::cadencer::HANDLER_MODULE,
            &crate::cadencer::VIEW_CALLS,
        ),
        (
            team_github_connector::HANDLER_MODULE,
            &team_github_connector::VIEW_CALLS,
        ),
        (
            team_agentic_resources::HANDLER_MODULE,
            &team_agentic_resources::VIEW_CALLS,
        ),
        (
            team_task_registry::HANDLER_MODULE,
            &team_task_registry::VIEW_CALLS,
        ),
    ]
}

/// What the page leg needs on an agent to draw its html view: `reflect`, which the page
/// reads the view from, then the view's own calls.
pub fn page_verbs(calls: &[&'static str]) -> Vec<&'static str> {
    let mut v = vec!["reflect"];
    v.extend(calls.iter().filter(|c| **c != "reflect"));
    v
}

/// Grant the dispatcher the read verbs each declared agent's declaration names, and the
/// team page's leg `reflect` plus the calls of each html view ([`page_views`]), on both
/// ends through [`crate::tree::grant_to`]: never `*`, verbs another grant put there stay.
/// Idempotent. Returns how many agents got a grant written.
pub fn grant_views(kernel: &Arc<Kernel>) -> usize {
    let mut written = 0;
    for (id, d) in declared_agents(kernel) {
        written += usize::from(crate::tree::grant_to(
            kernel,
            crate::tree::DISPATCHER_ID,
            id.as_str(),
            &read_verbs(&d),
        ));
    }
    let mut page: Vec<(AgentId, Vec<&str>)> = Vec::new();
    for e in kernel.agents.iter() {
        let a = e.value();
        let Some(m) = a.handler_module.as_deref() else {
            continue;
        };
        if let Some((_, calls)) = page_views().iter().find(|(h, _)| *h == m) {
            page.push((a.id.clone(), page_verbs(calls)));
        }
    }
    page.sort_by(|a, b| a.0 .0.cmp(&b.0 .0));
    for (id, verbs) in &page {
        written += usize::from(crate::tree::grant_to(
            kernel,
            crate::page::WS_ID,
            id.as_str(),
            verbs,
        ));
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_boot::{as_caller, boot_at, Seed};
    use fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
    use fantastic_kernel::Item;

    fn decl(module: &str) -> Decl {
        decl_for(module).unwrap_or_else(|| panic!("no declaration for {module}"))
    }

    #[test]
    fn declarations_cover_every_team_kind() {
        let all = decls();
        let kinds: Vec<&str> = all.iter().map(|d| d.kind).collect();
        for k in ["cadencer", "github", "git", "dispatcher"] {
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

    #[tokio::test(flavor = "multi_thread")]
    async fn declared_verbs_are_read_only() {
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
        let dir = tempfile::tempdir().unwrap();
        let booted = boot_at(dir.path(), Seed::Stock, &[], None).await;
        let reg = &booted.bundles;
        for d in decls() {
            let bundle = reg
                .get(d.handler_module)
                .unwrap_or_else(|| panic!("{} is not registered", d.handler_module));
            for s in &d.sections {
                assert!(
                    !writes.contains(&s.verb) && !s.verb.starts_with("worktree_") && s.verb != "*",
                    "{}: {} writes",
                    d.kind,
                    s.verb
                );
                assert!(
                    s.verb == "reflect" || bundle.verbs().iter().any(|v| v.name == s.verb),
                    "{}: `{}` is not a verb of {}",
                    d.kind,
                    s.verb,
                    d.handler_module
                );
            }
        }
        booted.stop().await;
    }

    fn cadencer_status() -> Value {
        // The shape of the cadencer's real `status` reply (cadencer.rs, "status").
        json!({
            "settings": {"enabled": true, "cadence": 300, "stall_after": 1800, "quiet": false, "off": []},
            "ticks": 7, "last_beat_s": 1760000100u64, "next_beat_s": 1760000400u64,
            "digests_sent": 3, "delivery_errors": 0,
            "digest_bytes_per_hour": 1200,
            "load": {
                "workers": {"busy": 2, "idle": 1, "stalled": 4},
                "groups": {"g": {"busy": 2, "idle": 1, "stalled": 4, "driver": "d"}},
                "tasks": {"held": 2, "in_review": 1, "done": 9, "blocked": 0, "queued": 5},
                "done_last_hour": 2,
                "digest_bytes_per_hour": 1200}
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

    #[tokio::test(flavor = "multi_thread")]
    async fn the_cadencers_real_status_reply_renders() {
        // The reply a live cadencer gives, not a hand-written fixture.
        let dir = tempfile::tempdir().unwrap();
        let kernel = boot_at(dir.path(), Seed::Stock, &[], None).await;
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
        let status = out[0].result.clone();
        kernel.stop().await;
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

    /// A reply for `d` with `plant` under every key a field does not name.
    fn planted_reply(d: &Decl, plant: &str) -> Value {
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
        reply["events"] = json!([{"unlisted": plant}]);
        reply["recent_calls"] = json!([{"unlisted": plant}]);
        reply["children"] = json!([{"unlisted": plant}]);
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
                // The token is only in keys no field names.
                let replies: Vec<Value> = d.sections.iter().map(|_| planted_reply(&d, t)).collect();
                let m = model_of(&d, &replies);
                assert!(!m.to_string().contains(t), "{}: {t} leaked: {m}", d.kind);
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

    // ---- records: grant_views ----

    async fn mk(kernel: &Arc<Kernel>, rec: Value) {
        let mut rec = rec;
        rec["type"] = json!("create_agent");
        rec["parent_id"] = json!(DEFAULT_ROOT_ID);
        let out = kernel.send(vec![Item::new(DEFAULT_ROOT_ID, rec)]).await;
        assert!(out[0].result.get("error").is_none(), "{}", out[0].result);
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
    async fn grant_views_names_read_verbs_not_star() {
        let dir = tempfile::tempdir().unwrap();
        let kernel = boot_at(dir.path(), Seed::Stock, &[], None).await;
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
            json!({"id": "gh", "handler_module": team_github_connector::HANDLER_MODULE,
                "app": {"credentials": {"store": "credentials", "ref": "github/app"}},
                "repos": ["o/r"], "directive": "Push a work branch.",
                "callable_by": {"dispatcher": "*"}}),
        )
        .await;
        assert!(grant_views(&kernel) > 0);
        let cc = field_of(&kernel, "dispatcher", "can_call");
        assert_eq!(cc["cad"], json!(["status"]), "{cc}");
        let cad = field_of(&kernel, "cad", "callable_by");
        assert_eq!(cad["dispatcher"], json!(["status"]), "{cad}");
        // A star the operator wrote is never narrowed.
        assert_eq!(
            field_of(&kernel, "gh", "callable_by")["dispatcher"],
            json!("*")
        );
        // Idempotent.
        assert_eq!(grant_views(&kernel), 0);
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn grant_views_does_not_grant_the_removed_view() {
        let dir = tempfile::tempdir().unwrap();
        let kernel = boot_at(dir.path(), Seed::Stock, &[], None).await;
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
            json!({"id": "cad", "handler_module": crate::cadencer::HANDLER_MODULE}),
        )
        .await;
        // the dispatcher's own boot granted its read verbs on the others; again is a no-op
        assert_eq!(grant_views(&kernel), 0, "already granted at boot");
        let cad_by = field_of(&kernel, "cad", "callable_by");
        assert!(cad_by.get("dispatcher").is_some(), "{cad_by}");
        for entry in kernel.agents.iter() {
            let meta = entry.meta.read().unwrap().clone();
            assert!(
                meta["callable_by"].get("dispatcher_view").is_none(),
                "{}: {meta:?}",
                entry.id.0
            );
        }
        assert!(field_of(&kernel, "dispatcher_view", "can_call")
            .get("cad")
            .is_none());
        kernel.stop().await;
    }

    #[test]
    fn page_verbs_lead_with_reflect_once() {
        assert_eq!(
            page_verbs(&["status", "set"]),
            vec!["reflect", "status", "set"]
        );
        assert_eq!(
            page_verbs(&["reflect", "pr_status"]),
            vec!["reflect", "pr_status"]
        );
        assert_eq!(page_verbs(&[]), vec!["reflect"]);
    }

    /// The team page reads a view with `reflect` AS ITS LEG (`team.html`: `send(id, {type:
    /// 'reflect'})` over the `team_ws` socket), so the leg needs `reflect` and the view's calls
    /// on each agent whose view it draws, and nothing more. The reflect here is sent as the
    /// leg, not as the operator: the gate is the one the page meets.
    #[tokio::test(flavor = "multi_thread")]
    async fn grant_views_lets_the_page_leg_reflect_each_html_view_and_call_it_and_no_more() {
        let dir = tempfile::tempdir().unwrap();
        let kernel = boot_at(dir.path(), Seed::Stock, &[], None).await;
        let leg = crate::page::WS_ID;
        mk(
            &kernel,
            json!({"id": "cad", "handler_module": crate::cadencer::HANDLER_MODULE}),
        )
        .await;
        mk(
            &kernel,
            json!({"id": "gh", "handler_module": team_github_connector::HANDLER_MODULE,
                "app": {"credentials": {"store": "credentials", "ref": "github/app"}},
                "repos": ["o/r"], "directive": "Push a work branch."}),
        )
        .await;
        mk(
            &kernel,
            json!({"id": "res", "handler_module": team_agentic_resources::HANDLER_MODULE}),
        )
        .await;
        mk(
            &kernel,
            json!({"id": "tasks", "handler_module": team_task_registry::HANDLER_MODULE}),
        )
        .await;
        // the leg comes last: the cadencer's boot (which grants the views) found no leg yet
        mk(
            &kernel,
            json!({"id": leg, "handler_module": "ram_store.tools"}),
        )
        .await;
        let views: [(&str, &[&str]); 4] = [
            ("cad", &crate::cadencer::VIEW_CALLS),
            ("gh", &team_github_connector::VIEW_CALLS),
            ("res", &team_agentic_resources::VIEW_CALLS),
            ("tasks", &team_task_registry::VIEW_CALLS),
        ];
        // before the grant the leg's reflect is refused by the gate: the view never mounts
        for (id, _) in views {
            let r = as_caller(&kernel, leg, id, json!({"type": "reflect"})).await;
            assert_eq!(r["reason"], "unauthorized", "{id}: {r}");
        }
        assert!(grant_views(&kernel) > 0);
        let cc = field_of(&kernel, leg, "can_call");
        assert_eq!(cc["cad"], json!(["reflect", "status", "set"]), "{cc}");
        assert_eq!(cc["gh"], json!(["reflect", "pr_status"]), "{cc}");
        assert_eq!(cc["tasks"], json!(["reflect", "list", "get"]), "{cc}");
        for (id, calls) in views {
            let want = json!(page_verbs(calls));
            assert_eq!(cc[id], want, "{cc}");
            assert_eq!(field_of(&kernel, id, "callable_by")[leg], want, "{id}");
            // no spawn, no star
            let v = cc[id].to_string();
            assert!(!v.contains("spawn") && !v.contains('*'), "{id}: {v}");
            // sent as the leg, the reflect is admitted and carries the view, whose calls are
            // the verbs granted
            let r = as_caller(&kernel, leg, id, json!({"type": "reflect"})).await;
            assert_eq!(r["view"]["kind"], "html", "{id}: {r}");
            assert_eq!(r["view"]["calls"][id], json!(calls), "{id}: {r}");
            // and the view's own calls pass the gate as the leg too
            let last = calls[calls.len() - 1];
            let c = as_caller(&kernel, leg, id, json!({"type": last})).await;
            assert_ne!(c["reason"], "unauthorized", "{id} {last}: {c}");
        }
        // an agent without an html view gets nothing
        assert!(cc.get("dispatcher").is_none(), "{cc}");
        // Idempotent.
        assert_eq!(grant_views(&kernel), 0);
        kernel.stop().await;
    }
}
