//! The views of the team's own service agents (the queue, the board, the github
//! connector and the dispatcher) for the dispatcher page's right pane. Each function
//! is pure: it takes a reply a service already gave and returns one model
//! `{kind, title, summary: [text], sections: [{name, columns: [text], rows: [[text]]}]}`.
//! The page draws every value as text (`textContent`), never as markup.
//!
//! The github view is built from a whitelist of the connector's `reflect` reply, never
//! from unknown keys, and the finished model is scanned for secrets.

use serde_json::{json, Value};

use crate::dispatcher::{agent_list, plan_of};

/// The view kind of a team service's handler module, if it has a view here. The queue
/// is a plain store and is named by the board record's `store`, not by a module.
pub fn service_kind(handler_module: &str) -> Option<&'static str> {
    match handler_module {
        "team_board.tools" => Some("board"),
        "team_github.tools" => Some("github"),
        "team_dispatcher.tools" => Some("dispatcher"),
        _ => None,
    }
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

fn or_dash(s: String) -> String {
    if s.is_empty() {
        "-".into()
    } else {
        s
    }
}

fn section(name: &str, columns: &[&str], rows: Vec<Vec<String>>) -> Value {
    json!({"name": name, "columns": columns, "rows": rows})
}

fn model(kind: &str, title: &str, summary: Vec<String>, sections: Vec<Value>) -> Value {
    json!({"kind": kind, "title": title, "summary": summary, "sections": sections})
}

/// A model with one "unavailable" row, for a service that did not answer.
pub fn unavailable(kind: &str, title: &str) -> Value {
    model(
        kind,
        title,
        vec!["unavailable".into()],
        vec![section(
            "status",
            &["state"],
            vec![vec!["unavailable".into()]],
        )],
    )
}

/// `Ns`, `Nm` or `Nh` for an age in seconds.
pub fn age_text(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

fn tasks_in<'a>(board: &'a Value, status: &str) -> &'a [Value] {
    board["tasks"][status]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// The queue: queued, taken and done tasks with title, holder and age. Failed tasks
/// are counted in the summary only.
pub fn queue_view(board: &Value, now: u64) -> Value {
    let rows_of = |status: &str| -> Vec<Vec<String>> {
        tasks_in(board, status)
            .iter()
            .map(|t| {
                let title = match text(&t["title"]) {
                    s if s.is_empty() => text(&t["id"]),
                    s => s,
                };
                let holder = t["claimers"]
                    .as_array()
                    .map(|a| a.iter().map(text).collect::<Vec<_>>().join(", "))
                    .unwrap_or_default();
                let age = match t["updated"].as_u64() {
                    Some(u) => age_text(now.saturating_sub(u)),
                    None => String::new(),
                };
                vec![or_dash(title), or_dash(holder), or_dash(age)]
            })
            .collect()
    };
    let cols = ["title", "holder", "age"];
    let n = |s: &str| tasks_in(board, s).len();
    model(
        "queue",
        "Queue",
        vec![format!(
            "{} queued, {} taken, {} done, {} failed",
            n("pending"),
            n("in_work"),
            n("done"),
            n("failed")
        )],
        vec![
            section("queued", &cols, rows_of("pending")),
            section("taken", &cols, rows_of("in_work")),
            section("done", &cols, rows_of("done")),
        ],
    )
}

/// The board: in roles mode the lead's agent tree and the registry's tasks; without
/// `roles` the board's own agents and the task counts by state.
pub fn board_view(board: &Value) -> Value {
    if board["roles"].is_object() {
        let roles = &board["roles"];
        let agents: Vec<Vec<String>> = agent_list(roles)
            .iter()
            .map(|a| {
                let pad = "  ".repeat(a["indent"].as_u64().unwrap_or(0) as usize);
                vec![
                    format!("{pad}{}", text(&a["label"])),
                    or_dash(text(&a["role"])),
                    or_dash(text(&a["state"])),
                ]
            })
            .collect();
        let plan = plan_of(roles);
        let tasks: Vec<Vec<String>> = plan["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| {
                vec![
                    or_dash(text(&t["id"])),
                    or_dash(text(&t["source"])),
                    or_dash(text(&t["holder"])),
                    or_dash(text(&t["status"])),
                ]
            })
            .collect();
        let mut summary = vec![format!("{} agents, {} tasks", agents.len(), tasks.len())];
        let f = &roles["fleet"];
        if let Some(r) = f["running"].as_u64() {
            let max = f["max"]
                .as_u64()
                .map(|m| m.to_string())
                .unwrap_or_else(|| "no cap".into());
            summary.push(format!("fleet {r}/{max} running"));
        }
        return model(
            "board",
            "Board",
            summary,
            vec![
                section("agents", &["agent", "role", "state"], agents),
                section("tasks", &["id", "source", "holder", "status"], tasks),
            ],
        );
    }
    let agents: Vec<Vec<String>> = board["agents"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(id, a)| {
            vec![
                id.clone(),
                or_dash(text(&a["state"])),
                or_dash(text(&a["cost"])),
            ]
        })
        .collect();
    let counts: Vec<Vec<String>> = ["pending", "in_work", "done", "failed"]
        .iter()
        .map(|s| vec![s.to_string(), tasks_in(board, s).len().to_string()])
        .collect();
    model(
        "board",
        "Board",
        vec![format!("{} agents", agents.len())],
        vec![
            section("agents", &["agent", "state", "cost"], agents),
            section("tasks by state", &["state", "count"], counts),
        ],
    )
}

/// `owner/name` or `owner/name#N`, nothing else.
fn is_target(s: &str) -> bool {
    let (repo, num) = match s.split_once('#') {
        Some((r, n)) => (r, Some(n)),
        None => (s, None),
    };
    if let Some(n) = num {
        if n.is_empty() || n.len() > 9 || !n.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
    }
    match repo.split_once('/') {
        Some((o, n)) => {
            let ok = |p: &str, extra: &[u8]| {
                !p.is_empty()
                    && p.len() <= 100
                    && p.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || extra.contains(&b))
            };
            ok(o, b"-") && ok(n, b"-._")
        }
        None => false,
    }
}

/// `ok`, `refused`, `unreachable` or `error <3 digits>`, nothing else.
fn is_result(s: &str) -> bool {
    matches!(s, "ok" | "refused" | "unreachable")
        || s.strip_prefix("error ")
            .is_some_and(|c| c.len() == 3 && c.bytes().all(|b| b.is_ascii_digit()))
}

/// True when the text looks like a credential: the secrets guard's rules, plus the
/// GitHub token prefixes for installation, user, refresh and OAuth tokens.
fn leaks(s: &str) -> bool {
    if !secrets_guard::scan::scan_text(s).is_empty() {
        return true;
    }
    ["ghs_", "ghu_", "ghr_", "gho_", "Bearer ", "eyJ"]
        .iter()
        .any(|p| s.contains(p))
}

/// The github connector: repos, the recent calls and the permission and rate state,
/// from a whitelist of the connector's `reflect` reply.
pub fn github_view(reflect: &Value) -> Value {
    let repos: Vec<String> = reflect["repos"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r.as_str())
        .filter(|r| is_target(r) && !r.contains('#'))
        .map(str::to_string)
        .collect();
    let api = reflect["api_url"]
        .as_str()
        .filter(|u| u.len() <= 200 && !u.contains('@') && !leaks(u))
        .unwrap_or("-")
        .to_string();
    let mut refused = 0usize;
    let calls: Vec<Vec<String>> = reflect["recent_calls"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let verb = c["verb"]
                .as_str()
                .filter(|v| {
                    !v.is_empty()
                        && v.len() <= 40
                        && v.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                })
                .unwrap_or("-");
            let target = c["target"].as_str().filter(|t| is_target(t)).unwrap_or("-");
            let result = c["result"].as_str().filter(|r| is_result(r)).unwrap_or("-");
            if result == "refused" {
                refused += 1;
            }
            vec![verb.to_string(), target.to_string(), result.to_string()]
        })
        .collect();
    let rate = match (
        reflect["rate"]["limit"].as_u64(),
        reflect["rate"]["remaining"].as_u64(),
        reflect["rate"]["reset"].as_u64(),
    ) {
        (Some(l), Some(r), Some(t)) => format!("{r}/{l} left, resets at {t}"),
        _ => "none yet".to_string(),
    };
    let summary = vec![
        format!("{} repos, {} recent calls", repos.len(), calls.len()),
        format!("rate {rate}"),
    ];
    let allow = if repos.is_empty() {
        "none".to_string()
    } else {
        repos.join(", ")
    };
    let calls = if calls.is_empty() {
        vec![vec!["none yet".into(), "-".into(), "-".into()]]
    } else {
        calls
    };
    let m = model(
        "github",
        "GitHub",
        summary,
        vec![
            section(
                "repos",
                &["repo", "api"],
                repos.iter().map(|r| vec![r.clone(), api.clone()]).collect(),
            ),
            section("recent calls", &["verb", "target", "result"], calls),
            section(
                "state",
                &["what", "value"],
                vec![
                    vec!["rate".into(), rate],
                    vec![
                        "permissions".into(),
                        format!("allowlisted repos: {allow}; drafts only; {refused} refused"),
                    ],
                ],
            ),
        ],
    );
    // Last step: nothing that looks like a credential leaves in the model.
    if leaks(&m.to_string()) {
        return unavailable("github", "GitHub");
    }
    m
}

/// The dispatcher: its workers, its lanes (the groups), and the agents running over the cap.
pub fn dispatcher_view(sidebar: &Value) -> Value {
    let mut lanes = Vec::new();
    let mut workers = Vec::new();
    let mut take = |items: &Value| -> (usize, [usize; 3]) {
        let mut n = 0;
        let mut c = [0usize; 3];
        for it in items.as_array().into_iter().flatten() {
            if it["kind"] != "worker" {
                continue;
            }
            n += 1;
            let st = text(&it["state"]);
            if st == "busy" {
                c[0] += 1;
            } else if st.starts_with("stalled") {
                c[2] += 1;
            } else {
                c[1] += 1;
            }
            workers.push(vec![
                or_dash(text(&it["id"])),
                or_dash(st),
                or_dash(text(&it["task"])),
                it["minutes"]
                    .as_u64()
                    .map(|m| format!("{m} min"))
                    .unwrap_or_else(|| "-".into()),
            ]);
        }
        (n, c)
    };
    for g in sidebar["groups"].as_array().into_iter().flatten() {
        let (n, c) = take(&g["items"]);
        lanes.push(vec![
            or_dash(text(&g["id"])),
            n.to_string(),
            format!("{} busy, {} idle, {} stalled", c[0], c[1], c[2]),
        ]);
    }
    let (n, c) = take(&sidebar["ungrouped"]);
    if n > 0 {
        lanes.push(vec![
            "(ungrouped)".into(),
            n.to_string(),
            format!("{} busy, {} idle, {} stalled", c[0], c[1], c[2]),
        ]);
    }
    let mut summary = vec![format!(
        "{} workers in {} lanes",
        workers.len(),
        lanes.len()
    )];
    let f = &sidebar["fleet"];
    if let Some(r) = f["running"].as_u64() {
        let max = f["max"]
            .as_u64()
            .map(|m| m.to_string())
            .unwrap_or_else(|| "no cap".into());
        summary.push(format!("agents {r}/{max}"));
    }
    model(
        "dispatcher",
        "Dispatcher",
        summary,
        vec![
            section("workers", &["worker", "state", "task", "idle"], workers),
            section("lanes", &["lane", "members", "load"], lanes),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(m: &Value, i: usize) -> Vec<Vec<String>> {
        m["sections"][i]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                r.as_array()
                    .unwrap()
                    .iter()
                    .map(|c| c.as_str().unwrap().to_string())
                    .collect()
            })
            .collect()
    }
    fn row(cells: &[&str]) -> Vec<String> {
        cells.iter().map(|c| c.to_string()).collect()
    }
    fn names(m: &Value) -> Vec<String> {
        m["sections"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn queue_view_sorts_tasks_into_queued_taken_done_with_title_holder_age() {
        let board = json!({"tasks": {
            "pending": [{"id": "o/r#1", "title": "first", "claimers": [], "updated": 990}],
            "in_work": [{"id": "o/r#2", "title": "second", "claimers": ["w1", "w2"], "updated": 700}],
            "done": [{"id": "o/r#3", "title": "third", "claimers": ["w1"], "updated": 0}],
            "failed": []}});
        let m = queue_view(&board, 1000);
        assert_eq!(m["kind"], "queue");
        assert_eq!(names(&m), ["queued", "taken", "done"]);
        assert_eq!(
            m["sections"][0]["columns"],
            json!(["title", "holder", "age"])
        );
        assert_eq!(rows(&m, 0), vec![row(&["first", "-", "10s"])]);
        assert_eq!(rows(&m, 1), vec![row(&["second", "w1, w2", "5m"])]);
        assert_eq!(rows(&m, 2), vec![row(&["third", "w1", "16m"])]);
    }

    #[test]
    fn queue_view_age_is_saturating_seconds_minutes_hours() {
        assert_eq!(age_text(0), "0s");
        assert_eq!(age_text(59), "59s");
        assert_eq!(age_text(90), "1m");
        assert_eq!(age_text(7200), "2h");
        let board = json!({"tasks": {"pending": [{"title": "t", "updated": 500}]}});
        assert_eq!(
            rows(&queue_view(&board, 100), 0),
            vec![row(&["t", "-", "0s"])]
        );
        assert_eq!(
            rows(&queue_view(&board, 7700), 0),
            vec![row(&["t", "-", "2h"])]
        );
    }

    #[test]
    fn queue_view_empty_board_has_three_empty_sections_and_zero_counts() {
        let m = queue_view(&json!({}), 5);
        assert_eq!(names(&m).len(), 3);
        for i in 0..3 {
            assert!(rows(&m, i).is_empty());
        }
        assert_eq!(m["summary"][0], "0 queued, 0 taken, 0 done, 0 failed");
    }

    #[test]
    fn queue_view_counts_failed_in_summary_but_has_no_failed_section() {
        let board = json!({"tasks": {"failed": [{"title": "x"}, {"title": "y"}]}});
        let m = queue_view(&board, 5);
        assert_eq!(m["summary"][0], "0 queued, 0 taken, 0 done, 2 failed");
        assert!(!names(&m).iter().any(|n| n.contains("fail")));
    }

    fn roles_board() -> Value {
        json!({"tasks": {}, "agents": {}, "roles": {
            "lead": "lead",
            "fleet": {"running": 2, "max": 5},
            "tree": {"id": "lead", "role": "project_lead", "state": "active", "children": [
                {"id": "c1", "role": "coder", "state": "active", "children": [
                    {"id": "c2", "role": "reviewer", "state": "active", "children": []}]}]},
            "agents": [],
            "tasks": [{"id": "t1", "source": "o/r#1", "holder": "c1", "status": "claimed"},
                      {"id": "t2", "source": "o/r#2", "holder": {"subtree": "c1"}, "status": "open"}]}})
    }

    #[test]
    fn board_view_roles_mode_lists_agent_tree_and_registry_tasks() {
        let m = board_view(&roles_board());
        assert_eq!(names(&m), ["agents", "tasks"]);
        assert_eq!(
            rows(&m, 0),
            vec![
                row(&["Lead", "project_lead", "active"]),
                row(&["  coder", "coder", "active"]),
                row(&["    reviewer", "reviewer", "active"]),
            ]
        );
        assert_eq!(
            rows(&m, 1),
            vec![
                row(&["t1", "o/r#1", "c1", "claimed"]),
                row(&["t2", "o/r#2", "subtree:c1", "open"]),
            ]
        );
        assert!(m["summary"].to_string().contains("fleet 2/5 running"));
    }

    #[test]
    fn board_view_without_roles_shows_agents_and_status_counts() {
        let board = json!({
            "tasks": {"pending": [{"id": "a"}], "in_work": [], "done": [{"id": "b"}, {"id": "c"}], "failed": []},
            "agents": {"w1": {"state": "coding", "cost": 1.5}}});
        let m = board_view(&board);
        assert_eq!(names(&m), ["agents", "tasks by state"]);
        assert_eq!(rows(&m, 0), vec![row(&["w1", "coding", "1.5"])]);
        assert_eq!(
            rows(&m, 1),
            vec![
                row(&["pending", "1"]),
                row(&["in_work", "0"]),
                row(&["done", "2"]),
                row(&["failed", "0"])
            ]
        );
        assert!(board_view(&json!({})).is_object());
    }

    fn reflect() -> Value {
        json!({
            "repos": ["acme/widgets", "acme/gadgets"],
            "api_url": "https://api.github.com",
            "recent_calls": [
                {"verb": "issue_view", "target": "acme/widgets#3", "result": "ok"},
                {"verb": "issues", "target": "evil/other", "result": "refused"},
                {"verb": "comment", "target": "acme/widgets#4", "result": "error 404"}],
            "rate": {"limit": 5000, "remaining": 4990, "reset": 1760000000u64}})
    }

    #[test]
    fn github_view_lists_repos_recent_calls_with_verb_target_result() {
        let m = github_view(&reflect());
        assert_eq!(m["kind"], "github");
        assert_eq!(names(&m), ["repos", "recent calls", "state"]);
        assert_eq!(
            rows(&m, 0)[0],
            row(&["acme/widgets", "https://api.github.com"])
        );
        assert_eq!(
            rows(&m, 1),
            vec![
                row(&["issue_view", "acme/widgets#3", "ok"]),
                row(&["issues", "evil/other", "refused"]),
                row(&["comment", "acme/widgets#4", "error 404"]),
            ]
        );
    }

    #[test]
    fn github_view_shows_rate_and_permission_state() {
        let m = github_view(&reflect());
        let st = rows(&m, 2);
        assert_eq!(
            st[0],
            row(&["rate", "4990/5000 left, resets at 1760000000"])
        );
        assert_eq!(
            st[1][1],
            "allowlisted repos: acme/widgets, acme/gadgets; drafts only; 1 refused"
        );
    }

    #[test]
    fn github_view_without_calls_or_rate_says_none_yet() {
        let m = github_view(&json!({"repos": ["a/b"], "rate": null, "recent_calls": []}));
        assert_eq!(rows(&m, 1), vec![row(&["none yet", "-", "-"])]);
        assert_eq!(rows(&m, 2)[0], row(&["rate", "none yet"]));
    }

    #[test]
    fn github_view_never_contains_a_token() {
        let tok = format!("{}{}{}", "gh", "s_", "A1b2".repeat(9));
        let pem = format!("-----BEGIN {} KEY-----", "RSA PRIVATE");
        let mut r = reflect();
        r["token"] = json!(tok);
        r["private_key"] = json!(pem);
        r["authorization"] = json!(format!("Bearer {tok}"));
        r["directive"] = json!(pem);
        r["recent_calls"] = json!([
            {"verb": "issue_view", "target": format!("acme/{tok}"), "result": "ok", "token": tok},
            {"verb": "issues", "target": "acme/widgets", "result": tok},
            {"verb": "issues", "target": pem, "result": "ok"}]);
        let s = github_view(&r).to_string();
        assert!(!s.contains(&tok), "{s}");
        assert!(!s.contains("PRIVATE"), "{s}");
        assert!(!s.contains("Bearer"), "{s}");
        assert!(secrets_guard::scan::scan_text(&s).is_empty());
    }

    #[test]
    fn github_view_drops_a_target_or_result_outside_the_whitelist_shape() {
        let r = json!({"repos": [], "recent_calls": [
            {"verb": "issues", "target": "<b>x</b>", "result": "weird"},
            {"verb": "issues", "target": "a/b#x", "result": "error 4040"},
            {"verb": "issues", "target": "a/b#7", "result": "unreachable"}]});
        let m = github_view(&r);
        assert_eq!(
            rows(&m, 1),
            vec![
                row(&["issues", "-", "-"]),
                row(&["issues", "-", "-"]),
                row(&["issues", "a/b#7", "unreachable"])
            ]
        );
    }

    fn sidebar() -> Value {
        json!({"team": "t",
            "groups": [{"id": "a", "items": [
                {"kind": "worker", "id": "w1", "state": "busy", "task": "o/r#1", "minutes": 4},
                {"kind": "subagents", "worker": "w1", "count": 0},
                {"kind": "worker", "id": "w2", "state": "idle", "task": null, "minutes": null},
                {"kind": "worker", "id": "w3", "state": "stalled 40 min", "task": "o/r#2", "minutes": 40}]}],
            "ungrouped": [],
            "fleet": {"running": 3, "max": 10}})
    }

    #[test]
    fn dispatcher_view_lists_workers_lanes_and_running_over_max() {
        let m = dispatcher_view(&sidebar());
        assert_eq!(names(&m), ["workers", "lanes"]);
        assert_eq!(rows(&m, 0)[0], row(&["w1", "busy", "o/r#1", "4 min"]));
        assert_eq!(rows(&m, 0)[1], row(&["w2", "idle", "-", "-"]));
        assert_eq!(
            rows(&m, 1),
            vec![row(&["a", "3", "1 busy, 1 idle, 1 stalled"])]
        );
        assert!(m["summary"].to_string().contains("agents 3/10"));
    }

    #[test]
    fn dispatcher_view_without_fleet_has_no_agent_count() {
        let mut s = sidebar();
        s.as_object_mut().unwrap().remove("fleet");
        assert!(!dispatcher_view(&s)["summary"]
            .to_string()
            .contains("agents "));
    }

    #[test]
    fn service_kind_maps_the_three_team_modules_and_nothing_else() {
        assert_eq!(service_kind("team_board.tools"), Some("board"));
        assert_eq!(service_kind("team_github.tools"), Some("github"));
        assert_eq!(service_kind("team_dispatcher.tools"), Some("dispatcher"));
        assert_eq!(service_kind("ram_store.tools"), None);
        assert_eq!(service_kind("team_git.tools"), None);
        assert_eq!(service_kind(""), None);
    }
}
