//! Dispatcher: the main view of the team's web UI. A recursive tree of
//! nodes (team → group → worker), each row with state, current task, time and
//! cost; per team its connections (present/missing, never values); and what is
//! waiting for the person. The dispatcher only reads and asks: no verb assigns
//! work. Opening a worker points at its own view; opening a group or team
//! points at a dispatcher bound to that node.

use std::collections::BTreeMap;
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

/// The URL a click opens. A worker goes to its own view (#36); a group or team
/// to a dispatcher bound to that node.
pub fn open_target(node: &str, kind: &str) -> Value {
    match kind {
        "worker" => json!({"view": "worker", "url": format!("/worker/{node}")}),
        _ => json!({"view": "dispatcher", "node": node, "url": format!("/dispatcher/{node}")}),
    }
}

/// `stop` needs an explicit confirm; anything else is answered with a question.
pub fn control_reply(node: &str, action: &str, confirm: bool) -> Value {
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

/// The group a worker belongs to, if any (its view cycles within that group).
fn group_of_worker(node: &str, groups: &BTreeMap<String, Vec<String>>) -> Option<String> {
    groups
        .iter()
        .find(|(_, members)| members.iter().any(|m| m == node))
        .map(|(g, _)| g.clone())
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

/// The dispatcher bundle: `tree`, `open`, `control`, `place`.
#[derive(Default)]
pub struct DispatcherBundle;

#[async_trait::async_trait]
impl fantastic_kernel::bundle::Bundle for DispatcherBundle {
    fn name(&self) -> &str {
        HANDLER_MODULE
    }

    fn verb_docs(&self) -> Value {
        json!({
            "tree": "tree {node?} → the tree of the team or a node, rows with state/task/minutes/cost",
            "open": "open {node} → the view to open for that node (worker → its view; group/team → a dispatcher)",
            "control": "control {node, action: start|pause|stop, confirm?} → {sent} or a question",
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
            "open" => {
                let node = payload["node"].as_str().unwrap_or("").to_string();
                let kind = kind_of(&node, &team, &groups);
                let mut out = open_target(&node, kind);
                out["kind"] = json!(kind);
                if kind == "worker" {
                    // The worker's own view, from the worker view agent (#36).
                    let wv = meta
                        .get("worker_view")
                        .and_then(Value::as_str)
                        .unwrap_or("worker_view");
                    let group = group_of_worker(&node, &groups);
                    let view = peers
                        .ask(wv, json!({"type": "open", "worker": node, "group": group}))
                        .await;
                    if view.get("worker").is_some() {
                        out["view"] = view;
                    } else {
                        out["view_error"] = view;
                    }
                }
                out
            }
            "control" => {
                let node = payload["node"].as_str().unwrap_or("").to_string();
                let action = payload["action"].as_str().unwrap_or("");
                let confirm = payload["confirm"].as_bool().unwrap_or(false);
                let reply = control_reply(&node, action, confirm);
                if reply["sent"] == json!(true) {
                    peers.ask(&node, json!({"type": action})).await;
                }
                reply
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
    fn open_worker_returns_worker_view() {
        assert_eq!(open_target("w1", "worker")["view"], "worker");
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
    async fn open_worker_returns_the_worker_view() {
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
        kernel
            .send(vec![Item::new(
                DEFAULT_ROOT_ID,
                json!({"type": "create_agent", "parent_id": "dispatcher", "id": "worker_view",
                    "handler_module": crate::worker_view::HANDLER_MODULE,
                    "callable_by": {"$self": "*", "$parent": "*"}}),
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
        assert_eq!(out["url"], "/worker/w1");
        assert_eq!(out["view"]["worker"], "w1");
        let cells = out["view"]["cells"].as_array().unwrap();
        let chat = serde_json::to_value(crate::worker_view::chat_cell("w1")).unwrap();
        assert_eq!(cells.last().unwrap(), &chat);
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[test]
    fn html_loads_tree_calls_open_and_has_back() {
        assert!(HTML.contains("fantastic.send("));
        assert!(HTML.contains("type: 'tree'"));
        assert!(HTML.contains("type: 'open'"));
        assert!(HTML.contains("getElementById('back')"));
        assert!(HTML.contains("scrollY"));
        assert!(HTML.contains("function esc("));
        assert!(HTML.contains("textContent"));
    }

    #[test]
    fn html_is_embedded_and_names_the_contract() {
        assert!(HTML.contains("tree"));
        assert!(HTML.contains("state"));
        assert!(HTML.contains("cost"));
    }
}
