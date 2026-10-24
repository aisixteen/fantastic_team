//! Dispatcher: the main view of the team's web UI. A sidebar (`sidebar`): per
//! group one flat list in which each worker is followed by its subagents
//! expander, and the subagents (kernel agents whose parent is that worker) show
//! only when the worker is expanded. A tree (`tree`) of nodes (team → group →
//! worker), each row with state, current task, time and cost; per team its
//! connections (present/missing, never values); and what is waiting for the
//! person. The dispatcher only reads and asks: no verb assigns work. Opening a
//! worker or a subagent returns a full-screen chat for that agent alone; opening
//! a group or team points at a dispatcher bound to that node.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::Serialize;
use serde_json::{json, Value};

use crate::puller::{KernelPeers, Peers};

/// The bundle's handler module name.
pub const HANDLER_MODULE: &str = "team_dispatcher.tools";

/// The dispatcher's html cell, served on `--port`.
pub const HTML: &str = include_str!("../web/dispatcher.html");

/// One worker as the dispatcher shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Row {
    /// The worker's agent id.
    pub id: String,
    /// `busy`, `idle`, `stalled N min`, `done`, `failed`.
    pub state: String,
    /// The current task (`repo#number`), if any.
    pub task: Option<String>,
    /// Minutes since the last change, when it is known.
    pub minutes: Option<u64>,
    /// Its cost so far.
    pub cost: f64,
}

/// One connection of a team: its name and whether it is set. Never its value.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Connection {
    /// The connection's name.
    pub name: String,
    /// `present` or `missing`.
    pub status: &'static str,
}

/// The state label for a row. A task that has not moved past `limit` minutes
/// reads as stalled.
pub fn state_label(task_status: &str, minutes: Option<u64>, limit: u64) -> String {
    if task_status == "in_work" {
        if let Some(m) = minutes {
            if m > limit {
                return format!("stalled {m} min");
            }
        }
        return "busy".into();
    }
    match task_status {
        "done" => "done".into(),
        "failed" => "failed".into(),
        _ => "idle".into(),
    }
}

/// Build the row for one worker from the board's snapshot.
pub fn row_of(worker: &str, board: &Value, now: u64, limit: u64) -> Row {
    let agent = &board["agents"][worker];
    let cost = agent["cost"].as_f64().unwrap_or(0.0);
    // The worker's current task: the first in-work task it claims.
    let mut task = None;
    let mut status = "pending";
    let mut updated = 0u64;
    if let Some(tasks) = board["tasks"]["in_work"].as_array() {
        for t in tasks {
            let claimed = t["claimers"]
                .as_array()
                .is_some_and(|c| c.iter().any(|x| x.as_str() == Some(worker)));
            if claimed {
                task = t["id"].as_str().map(str::to_string);
                status = "in_work";
                updated = t["updated"].as_u64().unwrap_or(0);
                break;
            }
        }
    }
    let minutes = (updated > 0).then(|| now.saturating_sub(updated) / 60);
    Row {
        id: worker.into(),
        state: state_label(status, minutes, limit),
        task,
        minutes,
        cost,
    }
}

/// The tree of a team: the team, its groups (each with its workers), and the
/// workers under no group. `groups` maps a group id to its worker ids.
pub fn build_tree(team: &str, groups: &BTreeMap<String, Vec<String>>, workers: &[Row]) -> Value {
    let find = |id: &str| workers.iter().find(|r| r.id == id).cloned();
    let mut grouped: Vec<String> = Vec::new();
    let mut children = Vec::new();
    for (gid, members) in groups {
        let kids: Vec<Value> = members
            .iter()
            .filter_map(|m| find(m))
            .map(|r| {
                grouped.push(r.id.clone());
                json!({"kind": "worker", "row": r})
            })
            .collect();
        children.push(json!({"kind": "group", "id": gid, "children": kids}));
    }
    for r in workers {
        if !grouped.contains(&r.id) {
            children.push(json!({"kind": "worker", "row": r}));
        }
    }
    json!({"kind": "team", "id": team, "children": children})
}

/// The connections of a team, from the names it declares. Only presence.
pub fn connections(names: &[String]) -> Vec<Connection> {
    names
        .iter()
        .map(|n| Connection {
            name: n.clone(),
            status: if std::env::var(n).is_ok() {
                "present"
            } else {
                "missing"
            },
        })
        .collect()
}

/// A kernel agent as the sidebar reads it: its parent, what it is, its status.
#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    /// The agent's parent id.
    pub parent: String,
    /// The agent's handler module (its tool or kind).
    pub kind: String,
    /// Its last status (`ok` when none is recorded).
    pub status: String,
}

/// A subagent of a worker: a kernel agent whose parent is that worker.
#[derive(Debug, Clone, PartialEq)]
pub struct Sub {
    /// The subagent's agent id.
    pub id: String,
    /// The subagent's tool or kind.
    pub kind: String,
    /// Its last status.
    pub status: String,
}

/// The direct children of `worker` in the agent table, in id order.
pub fn subagents_of(agents: &BTreeMap<String, Member>, worker: &str) -> Vec<Sub> {
    agents
        .iter()
        .filter(|(_, m)| m.parent == worker)
        .map(|(id, m)| Sub {
            id: id.clone(),
            kind: m.kind.clone(),
            status: m.status.clone(),
        })
        .collect()
}

/// The expanded set after a click on `worker`'s expander: only that worker flips.
pub fn toggle(expanded: &BTreeSet<String>, worker: &str) -> BTreeSet<String> {
    let mut out = expanded.clone();
    if !out.remove(worker) {
        out.insert(worker.to_string());
    }
    out
}

/// One worker's entries in a group's flat list: the worker, its subagents
/// expander, and (only when expanded) its subagents. All are siblings.
fn worker_items(
    r: &Row,
    subs: &BTreeMap<String, Vec<Sub>>,
    expanded: &BTreeSet<String>,
) -> Vec<Value> {
    let kids = subs.get(&r.id).map(Vec::as_slice).unwrap_or(&[]);
    let open = expanded.contains(&r.id);
    let mut out = vec![json!({
        "kind": "worker", "style": "worker", "id": r.id, "state": r.state,
        "task": r.task, "busy": r.state == "busy",
    })];
    out.push(json!({
        "kind": "subagents", "style": "subagent", "worker": r.id,
        "count": kids.len(), "expanded": open,
    }));
    if open {
        for s in kids {
            out.push(json!({
                "kind": "subagent", "style": "subagent", "worker": r.id,
                "id": s.id, "tool": s.kind, "status": s.status,
            }));
        }
    }
    out
}

/// The sidebar: per group one flat ordered list, and the workers under no group
/// in one more. Subagents start collapsed; `expanded` names the open workers.
/// `load` is the board's `load` object (`Null` when none came). A group with no
/// AI driver gets its raw counts as `load` {busy, idle, stalled}; a group with
/// a driver gets none, since its AI shows the summary. The team gets `load`
/// {tasks, done_last_hour} when the board gave one.
pub fn sidebar(
    team: &str,
    groups: &BTreeMap<String, Vec<String>>,
    rows: &[Row],
    subs: &BTreeMap<String, Vec<Sub>>,
    expanded: &BTreeSet<String>,
    load: &Value,
) -> Value {
    let mut grouped: Vec<String> = Vec::new();
    let mut gs = Vec::new();
    for (gid, members) in groups {
        let mut items = Vec::new();
        for m in members {
            if let Some(r) = rows.iter().find(|r| &r.id == m) {
                grouped.push(r.id.clone());
                items.extend(worker_items(r, subs, expanded));
            }
        }
        let mut group = json!({"id": gid, "items": items});
        let lg = &load["groups"][gid.as_str()];
        if lg.is_object() && lg["driver"].as_str().unwrap_or("").is_empty() {
            group["load"] = json!({
                "busy": lg["busy"],
                "idle": lg["idle"],
                "stalled": lg["stalled"],
            });
        }
        gs.push(group);
    }
    let mut loose = Vec::new();
    for r in rows {
        if !grouped.contains(&r.id) {
            loose.extend(worker_items(r, subs, expanded));
        }
    }
    let mut out = json!({"team": team, "groups": gs, "ungrouped": loose});
    if !load.is_null() {
        out["load"] = json!({
            "tasks": load["tasks"],
            "done_last_hour": load["done_last_hour"],
        });
    }
    out
}

/// The full-screen chat for one agent (a worker or a subagent). The chat targets
/// that agent alone; Back returns to the sidebar.
pub fn chat_open(node: &str) -> Value {
    json!({
        "view": "chat",
        "target": node,
        "fullscreen": true,
        "back": "sidebar",
        "cell": crate::worker_view::chat_cell(node),
    })
}

/// What a click shows: a worker or subagent opens its full-screen chat; a group
/// or team opens a dispatcher bound to that node.
pub fn open_target(node: &str, kind: &str) -> Value {
    match kind {
        "worker" => chat_open(node),
        _ => json!({"view": "dispatcher", "node": node, "url": format!("/dispatcher/{node}")}),
    }
}

/// The only actions the dispatcher sends a node. Anything else is refused here,
/// before anything is sent.
pub const CONTROL_ACTIONS: [&str; 3] = ["start", "pause", "stop"];

/// `stop` needs an explicit confirm; anything else is answered with a question.
pub fn control_reply(node: &str, action: &str, confirm: bool) -> Value {
    if !CONTROL_ACTIONS.contains(&action) {
        return json!({
            "error": format!("team_dispatcher: unknown control action '{action}'"),
            "sent": false,
        });
    }
    if node.is_empty() {
        return json!({"error": "team_dispatcher: control needs a node", "sent": false});
    }
    if action == "stop" && !confirm {
        return json!({
            "question": format!("Stop {node}? Send again with confirm: true."),
            "sent": false,
        });
    }
    json!({"sent": true, "node": node, "action": action})
}

/// What a node is: the team itself, one of its groups, or otherwise a worker.
/// `tree` and `open` both classify through this, so they agree.
pub fn kind_of(node: &str, team: &str, groups: &BTreeMap<String, Vec<String>>) -> &'static str {
    if node == team {
        "team"
    } else if groups.contains_key(node) {
        "group"
    } else {
        "worker"
    }
}

/// The subtree a click shows. The team is the whole tree; a group is the group
/// with its own workers (no team layer above it); a worker is its row. An id
/// that is no worker of the team is an error, not an empty tree.
pub fn subtree(
    team: &str,
    groups: &BTreeMap<String, Vec<String>>,
    rows: &[Row],
    node: &str,
) -> Value {
    match kind_of(node, team, groups) {
        "team" => build_tree(team, groups, rows),
        "group" => {
            let kids: Vec<Value> = groups
                .get(node)
                .map(Vec::as_slice)
                .unwrap_or(&[])
                .iter()
                .filter_map(|m| rows.iter().find(|r| &r.id == m))
                .map(|r| json!({"kind": "worker", "row": r}))
                .collect();
            json!({"kind": "group", "id": node, "children": kids})
        }
        _ => match rows.iter().find(|r| r.id == node) {
            Some(r) => json!({"kind": "worker", "row": r}),
            None => json!({"error": format!("dispatcher: unknown node '{node}'")}),
        },
    }
}

/// The rows of every worker the team knows: those in its groups and those on the board.
async fn rows_of(
    peers: &dyn Peers,
    board: &str,
    groups: &BTreeMap<String, Vec<String>>,
    limit: u64,
    now: u64,
) -> Vec<Row> {
    let snap = peers.ask(board, json!({"type": "board"})).await;
    let mut ids: Vec<String> = groups.values().flatten().cloned().collect();
    if let Some(agents) = snap["agents"].as_object() {
        for k in agents.keys() {
            if !ids.contains(k) {
                ids.push(k.clone());
            }
        }
    }
    ids.iter().map(|w| row_of(w, &snap, now, limit)).collect()
}

/// The kernel's agent table as the sidebar reads it: every agent with a parent.
fn agent_table(kernel: &fantastic_kernel::Kernel) -> BTreeMap<String, Member> {
    kernel
        .agents
        .iter()
        .filter_map(|e| {
            let a = e.value();
            let parent = a.parent_id.as_ref()?.0.clone();
            let status = a
                .meta
                .read()
                .ok()
                .and_then(|m| m.get("status").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_else(|| "ok".into());
            let member = Member {
                parent,
                kind: a.handler_module.clone().unwrap_or_default(),
                status,
            };
            Some((a.id.0.clone(), member))
        })
        .collect()
}

/// The workers the page has expanded, from the payload's `expanded` list.
fn expanded_of(payload: &Value) -> BTreeSet<String> {
    payload["expanded"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn groups_of(meta: &serde_json::Map<String, Value>) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    if let Some(obj) = meta.get("groups").and_then(Value::as_object) {
        for (g, v) in obj {
            let members = v
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            out.insert(g.clone(), members);
        }
    }
    out
}

/// The dispatcher bundle: `sidebar`, `tree`, `open`, `control`, `place`.
#[derive(Default)]
pub struct DispatcherBundle;

#[async_trait::async_trait]
impl fantastic_kernel::bundle::Bundle for DispatcherBundle {
    fn name(&self) -> &str {
        HANDLER_MODULE
    }

    fn verb_docs(&self) -> Value {
        json!({
            "sidebar": "sidebar {expanded?: [worker ids]} → per group one flat list: each worker, its subagents expander, and its subagents when expanded; a group with no AI driver also gets its raw load counts",
            "tree": "tree {node?} → the tree of the team or a node, rows with state/task/minutes/cost",
            "open": "open {node} → the view to open for that node (worker or subagent → full-screen chat; group/team → a dispatcher)",
            "control": "control {node, action: start|pause|stop, confirm?} → {sent, error?} (or a question for stop without confirm); the node's refusal or no reply is sent: false with its error",
            "place": "place {canvas} → puts the dispatcher cell on that canvas",
        })
    }

    async fn handle(
        &self,
        agent_id: &fantastic_kernel::AgentId,
        payload: &Value,
        kernel: &Arc<fantastic_kernel::Kernel>,
    ) -> Result<fantastic_kernel::bundle::Reply, fantastic_kernel::bundle::BundleError> {
        let t = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if t == "boot" {
            return Ok(None);
        }
        let meta = kernel
            .agents
            .get(agent_id)
            .map(|e| e.meta.read().expect("meta").clone())
            .unwrap_or_default();
        let board = meta
            .get("board")
            .and_then(Value::as_str)
            .unwrap_or("board")
            .to_string();
        let team = meta
            .get("team")
            .and_then(Value::as_str)
            .unwrap_or("team")
            .to_string();
        let limit = meta
            .get("stall_minutes")
            .and_then(Value::as_u64)
            .unwrap_or(30);
        let groups = groups_of(&meta);
        let peers = KernelPeers(kernel.clone());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let out = match t {
            "tree" => {
                let node = payload["node"].as_str().unwrap_or(&team).to_string();
                let rows = rows_of(&peers, &board, &groups, limit, now).await;
                subtree(&team, &groups, &rows, &node)
            }
            "sidebar" => {
                let rows = rows_of(&peers, &board, &groups, limit, now).await;
                let agents = agent_table(kernel);
                let subs: BTreeMap<String, Vec<Sub>> = rows
                    .iter()
                    .map(|r| (r.id.clone(), subagents_of(&agents, &r.id)))
                    .collect();
                let reply = peers.ask(&board, json!({"type": "rows"})).await;
                let load = reply.get("load").cloned().unwrap_or(Value::Null);
                sidebar(&team, &groups, &rows, &subs, &expanded_of(payload), &load)
            }
            "open" => {
                let node = payload["node"].as_str().unwrap_or("").to_string();
                let kind = kind_of(&node, &team, &groups);
                let mut out = open_target(&node, kind);
                out["kind"] = json!(kind);
                out
            }
            "control" => {
                let node = payload["node"].as_str().unwrap_or("").to_string();
                let action = payload["action"].as_str().unwrap_or("");
                let confirm = payload["confirm"].as_bool().unwrap_or(false);
                let reply = control_reply(&node, action, confirm);
                if reply["sent"] != json!(true) {
                    return Ok(Some(reply));
                }
                // The node's own answer decides: a refusal or no reply is not "sent".
                let peer = peers.ask(&node, json!({"type": action})).await;
                let peer_error = if peer.is_null() {
                    Some(json!(format!("team_dispatcher: no reply from '{node}'")))
                } else {
                    peer.get("error").filter(|e| !e.is_null()).cloned()
                };
                match peer_error {
                    Some(error) => {
                        json!({"sent": false, "node": node, "action": action, "error": error})
                    }
                    None => {
                        let mut out = reply;
                        out["reply"] = peer;
                        out
                    }
                }
            }
            "place" => {
                let canvas = payload["canvas"]
                    .as_str()
                    .unwrap_or("canvas_page")
                    .to_string();
                peers
                    .ask(&canvas, json!({"type": "create_agent", "id": format!("{agent_id}_view"), "handler_module": "html.tools", "html": HTML}))
                    .await
            }
            other => json!({"error": format!("team_dispatcher: unknown type '{other}'")}),
        };
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> Value {
        json!({
            "tasks": {"in_work": [{"id": "r#1", "claimers": ["w1"], "updated": 0}], "done": [], "pending": [], "failed": []},
            "agents": {"w1": {"state": "coding", "cost": 1.5}, "w2": {"state": "", "cost": 0.0}}
        })
    }

    #[test]
    fn tree_groups_workers_under_groups_under_teams() {
        let mut groups = BTreeMap::new();
        groups.insert("g1".to_string(), vec!["w1".to_string()]);
        let rows = vec![row_of("w1", &board(), 0, 30), row_of("w2", &board(), 0, 30)];
        let t = build_tree("team", &groups, &rows);
        assert_eq!(t["kind"], "team");
        assert_eq!(t["children"][0]["kind"], "group");
        assert_eq!(t["children"][0]["children"][0]["row"]["id"], "w1");
        assert_eq!(t["children"][1]["row"]["id"], "w2");
    }

    #[test]
    fn row_shows_state_task_time_cost() {
        let r = row_of("w1", &board(), 600, 30);
        assert_eq!(r.task.as_deref(), Some("r#1"));
        assert_eq!(r.cost, 1.5);
        assert!(r.minutes.is_none() || r.minutes.is_some());
    }

    #[test]
    fn stalled_row_names_minutes() {
        assert_eq!(state_label("in_work", Some(45), 30), "stalled 45 min");
        assert_eq!(state_label("in_work", Some(5), 30), "busy");
    }

    #[test]
    fn connections_never_contain_key_values() {
        std::env::set_var("TEAM58_TEST_SECRET", "hunter2");
        let c = connections(&["TEAM58_TEST_SECRET".to_string()]);
        let s = serde_json::to_string(&c).unwrap();
        assert!(!s.contains("hunter2"));
        assert_eq!(c[0].status, "present");
    }

    #[test]
    fn open_worker_or_subagent_returns_fullscreen_chat_targeting_only_it() {
        for id in ["w1", "s1"] {
            let v = open_target(id, "worker");
            assert_eq!(v["view"], "chat", "{id}");
            assert_eq!(v["fullscreen"], json!(true), "{id}");
            assert_eq!(v["target"], id, "{id}");
            assert_eq!(v["back"], "sidebar", "{id}");
            assert_eq!(v["cell"]["kind"], "chat", "{id}");
            assert_eq!(v["cell"]["target"], id, "{id}");
        }
    }

    #[test]
    fn toggle_flips_only_that_worker() {
        let empty = BTreeSet::new();
        let a = toggle(&empty, "w1");
        assert!(a.contains("w1"));
        assert!(!a.contains("w2"));
        let b = toggle(&a, "w1");
        assert!(b.is_empty());
    }

    fn members() -> BTreeMap<String, Member> {
        let m = |parent: &str, kind: &str| Member {
            parent: parent.into(),
            kind: kind.into(),
            status: "ok".into(),
        };
        let mut out = BTreeMap::new();
        out.insert("s1".to_string(), m("w1", "gh_exec"));
        out.insert("s2".to_string(), m("w1", "git"));
        out.insert("s3".to_string(), m("w2", "reviewer"));
        out.insert("x".to_string(), m("elsewhere", "git"));
        out
    }

    fn subs_of(rows: &[Row]) -> BTreeMap<String, Vec<Sub>> {
        rows.iter()
            .map(|r| (r.id.clone(), subagents_of(&members(), &r.id)))
            .collect()
    }

    fn group_items(sb: &Value, gid: &str) -> Vec<Value> {
        sb["groups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["id"] == gid)
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .clone()
    }

    #[test]
    fn subagents_of_lists_direct_children_only() {
        let s = subagents_of(&members(), "w1");
        let ids: Vec<&str> = s.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, vec!["s1", "s2"]);
        assert_eq!(s[0].kind, "gh_exec");
        assert_eq!(s[0].status, "ok");
    }

    #[test]
    fn sidebar_subagent_lists_start_collapsed() {
        let mut groups = BTreeMap::new();
        groups.insert("A".to_string(), vec!["w1".to_string(), "w2".to_string()]);
        let rows = rows_w();
        let subs = subs_of(&rows);
        let sb = sidebar(
            "team",
            &groups,
            &rows,
            &subs,
            &BTreeSet::new(),
            &Value::Null,
        );
        let items = group_items(&sb, "A");
        let exp = items
            .iter()
            .find(|i| i["kind"] == "subagents" && i["worker"] == "w1")
            .unwrap();
        assert_eq!(exp["expanded"], json!(false));
        assert_eq!(exp["count"], json!(2));
        assert!(!items.iter().any(|i| i["kind"] == "subagent"));
    }

    #[test]
    fn expanding_one_worker_keeps_others_collapsed() {
        let mut groups = BTreeMap::new();
        groups.insert("A".to_string(), vec!["w1".to_string(), "w2".to_string()]);
        let rows = rows_w();
        let subs = subs_of(&rows);
        let open = toggle(&BTreeSet::new(), "w1");
        let sb = sidebar("team", &groups, &rows, &subs, &open, &Value::Null);
        let items = group_items(&sb, "A");
        let shown: Vec<&Value> = items.iter().filter(|i| i["kind"] == "subagent").collect();
        assert_eq!(shown.len(), 2);
        assert!(shown.iter().all(|i| i["worker"] == "w1"));
        let w2 = items
            .iter()
            .find(|i| i["kind"] == "subagents" && i["worker"] == "w2")
            .unwrap();
        assert_eq!(w2["expanded"], json!(false));
        let again = toggle(&open, "w1");
        let sb2 = sidebar("team", &groups, &rows, &subs, &again, &Value::Null);
        assert!(!group_items(&sb2, "A")
            .iter()
            .any(|i| i["kind"] == "subagent"));
    }

    #[test]
    fn workers_and_subagents_are_siblings_with_different_style() {
        let mut groups = BTreeMap::new();
        groups.insert("A".to_string(), vec!["w1".to_string(), "w2".to_string()]);
        let rows = rows_w();
        let subs = subs_of(&rows);
        let open = toggle(&BTreeSet::new(), "w1");
        let sb = sidebar("team", &groups, &rows, &subs, &open, &Value::Null);
        let items = group_items(&sb, "A");
        // Flat: every item is a leaf of the one list, with no nested children.
        assert!(items.iter().all(|i| i.get("children").is_none()));
        let w1 = items
            .iter()
            .position(|i| i["kind"] == "worker" && i["id"] == "w1")
            .unwrap();
        let sub = items
            .iter()
            .position(|i| i["kind"] == "subagent" && i["id"] == "s1")
            .unwrap();
        assert!(w1 < sub);
        assert_eq!(items[w1]["style"], "worker");
        assert_eq!(items[sub]["style"], "subagent");
        assert_ne!(items[w1]["style"], items[sub]["style"]);
        assert_eq!(items[sub]["tool"], "gh_exec");
        assert_eq!(items[sub]["status"], "ok");
    }

    #[test]
    fn open_group_returns_nested_dispatcher() {
        assert_eq!(open_target("g1", "group")["view"], "dispatcher");
    }

    #[test]
    fn stop_without_confirm_asks_first() {
        let r = control_reply("w1", "stop", false);
        assert_eq!(r["sent"], json!(false));
        assert!(r["question"].as_str().is_some());
    }

    fn groups_a() -> BTreeMap<String, Vec<String>> {
        let mut g = BTreeMap::new();
        g.insert("A".to_string(), vec!["w1".to_string()]);
        g
    }

    fn rows_w() -> Vec<Row> {
        vec![row_of("w1", &board(), 0, 30), row_of("w2", &board(), 0, 30)]
    }

    #[test]
    fn tree_of_a_group_is_the_group_with_its_workers_no_fake_team() {
        let t = subtree("team", &groups_a(), &rows_w(), "A");
        assert_eq!(t["kind"], "group");
        assert_eq!(t["id"], "A");
        let kids = t["children"].as_array().unwrap();
        assert_eq!(kids.len(), 1);
        assert_eq!(kids[0]["row"]["id"], "w1");
    }

    #[test]
    fn tree_of_a_worker_is_its_row() {
        let t = subtree("team", &groups_a(), &rows_w(), "w2");
        assert_eq!(t["kind"], "worker");
        assert_eq!(t["row"]["id"], "w2");
    }

    #[test]
    fn open_classifies_team_group_worker_with_one_helper() {
        let g = groups_a();
        assert_eq!(kind_of("team", "team", &g), "team");
        assert_eq!(kind_of("A", "team", &g), "group");
        assert_eq!(kind_of("w1", "team", &g), "worker");
        assert_eq!(kind_of("nope", "team", &g), "worker");
        let e = subtree("team", &g, &rows_w(), "nope");
        assert!(e["error"].as_str().is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn open_worker_via_kernel_returns_fullscreen_chat() {
        use fantastic_kernel::bootstrap::{self, DEFAULT_ROOT_ID};
        use fantastic_kernel::Item;
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        // As the plan creates them: the view agent under the dispatcher.
        kernel
            .send(vec![Item::new(
                DEFAULT_ROOT_ID,
                json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "dispatcher",
                    "handler_module": HANDLER_MODULE, "can_call": {"worker_view": "*"}}),
            )])
            .await;
        let out = kernel
            .send(vec![Item::new(
                "dispatcher",
                json!({"type": "open", "node": "w1"}),
            )])
            .await
            .remove(0)
            .result;
        assert_eq!(out["kind"], "worker");
        assert_eq!(out["view"], "chat");
        assert_eq!(out["fullscreen"], json!(true));
        assert_eq!(out["target"], "w1");
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[test]
    fn html_loads_tree_calls_open_and_has_back() {
        assert!(HTML.contains("fantastic.send("));
        assert!(HTML.contains("type: 'open'"));
        assert!(HTML.contains("getElementById('back')"));
        assert!(HTML.contains("scrollY"));
        assert!(HTML.contains("textContent"));
    }

    #[test]
    fn html_sidebar_has_expander_back_and_remembers_state() {
        // The sidebar request, the subagent style, the expander label.
        assert!(HTML.contains("type: 'sidebar'"));
        // A group with no AI shows its raw load under its title.
        assert!(HTML.contains("g.load.busy"));
        assert!(HTML.contains(".sub {"));
        assert!(HTML.contains("subagents ("));
        // Back restores both the scroll and the expansion the level had.
        assert!(HTML.contains("function back("));
        assert!(HTML.contains("expanded"));
        assert!(HTML.contains("scrollTo("));
        // Remembered per worker, and wrapped so a blocked store cannot break the page.
        assert!(HTML.contains("localStorage"));
        assert!(HTML.contains("try {"));
        // Kernel values go in as text only: no innerHTML anywhere.
        assert!(!HTML.contains("innerHTML"));
    }

    #[test]
    fn sidebar_shows_raw_load_for_groups_without_ai_only() {
        let mut groups = BTreeMap::new();
        groups.insert("a".to_string(), vec!["w1".to_string()]);
        groups.insert("b".to_string(), vec!["w2".to_string()]);
        let rows = rows_w();
        let subs = subs_of(&rows);
        let load = json!({
            "groups": {
                "a": {"busy": 1, "idle": 2, "stalled": 3, "driver": ""},
                "b": {"busy": 4, "idle": 5, "stalled": 6, "driver": "drv"},
            },
            "tasks": {"held": 1, "in_review": 0, "done": 2, "blocked": 0, "queued": 3},
            "done_last_hour": 2,
        });
        let sb = sidebar("team", &groups, &rows, &subs, &BTreeSet::new(), &load);
        let group = |id: &str| {
            sb["groups"]
                .as_array()
                .unwrap()
                .iter()
                .find(|g| g["id"] == id)
                .unwrap()
                .clone()
        };
        let a = group("a");
        assert_eq!(a["load"]["busy"], 1);
        assert_eq!(a["load"]["idle"], 2);
        assert_eq!(a["load"]["stalled"], 3);
        assert!(group("b").get("load").is_none(), "{}", group("b"));
        assert_eq!(sb["load"]["tasks"]["queued"], 3);
        assert_eq!(sb["load"]["done_last_hour"], 2);
        assert!(!sb.to_string().contains("\"mode\""), "{sb}");
        // With no load from the board, the sidebar has no load keys at all.
        let bare = sidebar(
            "team",
            &groups,
            &rows,
            &subs,
            &BTreeSet::new(),
            &Value::Null,
        );
        assert!(bare.get("load").is_none());
        assert!(!group_items(&bare, "a").is_empty());
    }

    #[test]
    fn control_refuses_actions_outside_start_pause_stop() {
        for action in ["delete_agent", "reflect"] {
            let r = control_reply("w1", action, true);
            assert_eq!(r["sent"], json!(false), "{r}");
            assert!(r["error"].as_str().is_some(), "{r}");
        }
        for action in ["start", "pause"] {
            assert_eq!(control_reply("w1", action, false)["sent"], json!(true));
        }
        let stop = control_reply("w1", "stop", true);
        assert_eq!(stop["sent"], json!(true), "{stop}");
        assert_eq!(stop["action"], json!("stop"));
    }

    const PROBE: &str = "probe_test.tools";

    /// Records every message it gets. It refuses `pause` as an unknown verb.
    struct ProbeBundle {
        seen: Arc<std::sync::Mutex<Vec<Value>>>,
    }

    #[async_trait::async_trait]
    impl fantastic_kernel::bundle::Bundle for ProbeBundle {
        fn name(&self) -> &str {
            PROBE
        }

        async fn handle(
            &self,
            _agent_id: &fantastic_kernel::AgentId,
            payload: &Value,
            _kernel: &Arc<fantastic_kernel::Kernel>,
        ) -> Result<fantastic_kernel::bundle::Reply, fantastic_kernel::bundle::BundleError>
        {
            if payload["type"] == json!("boot") {
                return Ok(None);
            }
            self.seen.lock().unwrap().push(payload.clone());
            if payload["type"] == json!("pause") {
                return Ok(Some(json!({"error": "no such verb"})));
            }
            Ok(Some(json!({"ok": true})))
        }
    }

    /// A kernel booted the way `crate::start` boots, with the probe registered,
    /// a dispatcher that may call it, and the probe that may be called by it.
    async fn kernel_with_probe(
        dir: &std::path::Path,
        seen: Arc<std::sync::Mutex<Vec<Value>>>,
    ) -> Arc<fantastic_kernel::Kernel> {
        use fantastic_kernel::bootstrap::{self, BootstrapOptions, DEFAULT_ROOT_ID};
        use fantastic_kernel::Item;
        let mut reg = crate::registry();
        reg.register(PROBE, ProbeBundle { seen });
        let mut booted = bootstrap::bootstrap(reg, BootstrapOptions::one_shot(dir)).unwrap();
        fantastic_kernel::lock::acquire(dir).unwrap();
        booted.write_seed().unwrap();
        let kernel = booted.kernel.clone();
        crate::boot_kernel(&kernel).await.unwrap();
        for item in [
            json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "dispatcher",
                "handler_module": HANDLER_MODULE, "can_call": {"probe": "*"}}),
            json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "probe",
                "handler_module": PROBE, "callable_by": {"dispatcher": "*"}}),
        ] {
            kernel.send(vec![Item::new(DEFAULT_ROOT_ID, item)]).await;
        }
        kernel
    }

    async fn ask_kernel(kernel: &Arc<fantastic_kernel::Kernel>, to: &str, payload: Value) -> Value {
        kernel
            .send(vec![fantastic_kernel::Item::new(to, payload)])
            .await
            .remove(0)
            .result
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn control_via_kernel_does_not_relay_an_unknown_action() {
        use fantastic_kernel::bootstrap;
        let dir = tempfile::TempDir::new().unwrap();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let kernel = kernel_with_probe(dir.path(), seen.clone()).await;
        let out = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "control", "node": "probe", "action": "delete_agent"}),
        )
        .await;
        assert_eq!(out["sent"], json!(false), "{out}");
        assert!(out["error"].as_str().is_some(), "{out}");
        assert!(
            seen.lock().unwrap().is_empty(),
            "{:?}",
            seen.lock().unwrap()
        );
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn control_via_kernel_reports_a_peer_refusal() {
        use fantastic_kernel::bootstrap;
        let dir = tempfile::TempDir::new().unwrap();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let kernel = kernel_with_probe(dir.path(), seen.clone()).await;
        let out = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "control", "node": "probe", "action": "pause"}),
        )
        .await;
        assert_eq!(out["sent"], json!(false), "{out}");
        assert!(
            out["error"].as_str().unwrap_or("").contains("no such verb"),
            "{out}"
        );
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn control_via_kernel_stop_needs_confirm_then_reaches_the_node() {
        use fantastic_kernel::bootstrap;
        let dir = tempfile::TempDir::new().unwrap();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let kernel = kernel_with_probe(dir.path(), seen.clone()).await;
        let asked = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "control", "node": "probe", "action": "stop"}),
        )
        .await;
        assert_eq!(asked["sent"], json!(false), "{asked}");
        assert!(asked["question"].as_str().is_some(), "{asked}");
        assert!(seen.lock().unwrap().is_empty());
        let done = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "control", "node": "probe", "action": "stop", "confirm": true}),
        )
        .await;
        assert_eq!(done["sent"], json!(true), "{done}");
        assert_eq!(*seen.lock().unwrap(), vec![json!({"type": "stop"})]);
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    /// The text of one top-level JS function: from its header to the closing
    /// brace at column zero.
    fn js_fn(name: &str) -> &'static str {
        let start = HTML
            .find(&format!("function {name}("))
            .unwrap_or_else(|| panic!("no function {name}"));
        let end = HTML[start..].find("\n}\n").expect("closing brace") + start;
        &HTML[start..end]
    }

    #[test]
    fn html_has_per_worker_controls_that_confirm_stop() {
        assert!(HTML.contains("type: 'control'"));
        assert!(HTML.contains("data-control"));
        assert!(HTML.contains("confirm:"));
        assert!(HTML.contains("window.confirm("));
        // The buttons exist in the worker row only, never on a subagent row.
        let worker = js_fn("workerRow");
        assert!(worker.contains("dataset.control"), "{worker}");
        assert!(worker.contains("el('button'"), "{worker}");
        assert!(!js_fn("subRow").contains("dataset.control"));
        // The click handler routes a control button before it opens a node.
        let click = HTML
            .find("addEventListener('click'")
            .expect("click handler");
        let handler = &HTML[click..];
        let control_at = handler.find("[data-control]").expect("control in handler");
        let node_at = handler.find("[data-node]").expect("node in handler");
        assert!(control_at < node_at, "control must be routed first");
    }
}
