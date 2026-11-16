//! The views of the team's own service agents (the github connector and the dispatcher)
//! for the team page's right pane. Each function is pure: it takes a reply a service
//! already gave and returns one model
//! `{kind, title, summary: [text], sections: [{name, columns: [text], rows: [[text]]}]}`.
//! The page draws every value as text (`textContent`), never as markup.
//!
//! These stay page code, not declarations (`declared_views`): the dispatcher view reads
//! nested object-of-arrays replies (the sidebar's `groups[].items[]`), which the declared
//! `table` (a top-level list of rows) and `kv` cannot express, and the github view
//! whitelists and sanitizes each field (`is_target`, `is_result`) instead of printing
//! what the connector answers. The kinds a declaration can express (cadencer, git) are
//! built by `declared_views::model_of`. The task registry draws its own html view
//! (`task_registry::view_of`), which the page mounts from its `reflect`.
//!
//! The github view is built from a whitelist of the connector's `reflect` reply, never
//! from unknown keys.

use serde_json::{json, Value};

/// The view kind of a team service's handler module, if it has a view here.
pub fn service_kind(handler_module: &str) -> Option<&'static str> {
    match handler_module {
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

/// The github connector: repos, the recent calls, the permission and rate state and
/// the active auth mode (read-only), from a whitelist of the connector's `reflect` reply.
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
        .filter(|u| u.len() <= 200 && !u.contains('@'))
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
    let label = match reflect["auth"]["mode"].as_str() {
        Some("personal") => "personal token (env ref)",
        Some("app") => "GitHub App (env refs)",
        Some("local") => "local only",
        Some("invalid") => "credential unusable",
        _ => "unknown",
    };
    let note = reflect["auth"]["note"]
        .as_str()
        .filter(|n| n.len() <= 300)
        .map(str::to_string);
    let summary = vec![
        format!("{} repos, {} recent calls", repos.len(), calls.len()),
        format!("rate {rate}"),
        format!("auth: {label}"),
    ];
    let allow = if repos.is_empty() {
        "none".to_string()
    } else {
        repos.join(", ")
    };
    let mut state_rows = vec![
        vec!["rate".into(), rate],
        vec![
            "permissions".into(),
            format!("allowlisted repos: {allow}; drafts only; {refused} refused"),
        ],
        vec!["auth".into(), label.to_string()],
    ];
    if let Some(n) = note {
        state_rows.push(vec!["note".into(), n]);
    }
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
            section("state", &["what", "value"], state_rows),
        ],
    );
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
        summary.push(format!("agents {r}"));
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
    fn github_view_shows_the_auth_mode_and_the_local_note() {
        let note =
            "no GitHub credential defined: local work only (set `GH_TOKEN`, or an app id + key)";
        let mut r = reflect();
        r["auth"] = json!({"mode": "local", "note": note});
        let m = github_view(&r);
        let st = rows(&m, 2);
        assert!(st.contains(&row(&["auth", "local only"])), "{st:?}");
        assert!(st.contains(&row(&["note", note])), "{st:?}");
        assert_eq!(m["summary"][2], "auth: local only");
        r["auth"] = json!({"mode": "personal"});
        let st = rows(&github_view(&r), 2);
        assert!(st.contains(&row(&["auth", "personal token (env ref)"])));
        assert!(st.iter().all(|x| x[0] != "note"));
        r["auth"] = json!({"mode": "app"});
        let st = rows(&github_view(&r), 2);
        assert!(st.contains(&row(&["auth", "GitHub App (env refs)"])));
        let st = rows(&github_view(&reflect()), 2);
        assert!(st.contains(&row(&["auth", "unknown"])));
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
        r["auth"] = json!({"mode": "personal"});
        r["directive"] = json!(pem);
        r["recent_calls"] = json!([
            {"verb": "issue_view", "target": "acme/widgets", "result": "ok", "token": tok},
            {"verb": "issues", "target": "acme/widgets", "result": tok},
            {"verb": "issues", "target": pem, "result": "ok"}]);
        let s = github_view(&r).to_string();
        assert!(!s.contains(&tok), "{s}");
        assert!(!s.contains("PRIVATE"), "{s}");
        assert!(!s.contains("Bearer"), "{s}");
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
            "fleet": {"running": 3}})
    }

    #[test]
    fn dispatcher_view_lists_workers_lanes_and_running() {
        let m = dispatcher_view(&sidebar());
        assert_eq!(names(&m), ["workers", "lanes"]);
        assert_eq!(rows(&m, 0)[0], row(&["w1", "busy", "o/r#1", "4 min"]));
        assert_eq!(rows(&m, 0)[1], row(&["w2", "idle", "-", "-"]));
        assert_eq!(
            rows(&m, 1),
            vec![row(&["a", "3", "1 busy, 1 idle, 1 stalled"])]
        );
        assert!(m["summary"].to_string().contains("agents 3"));
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
    fn service_kind_maps_the_two_team_modules_and_nothing_else() {
        assert_eq!(service_kind("team_github.tools"), Some("github"));
        assert_eq!(service_kind("team_dispatcher.tools"), Some("dispatcher"));
        assert_eq!(service_kind("ram_store.tools"), None);
        assert_eq!(service_kind("team_git.tools"), None);
        assert_eq!(service_kind(""), None);
    }
}
