//! The plan: the spawned tree and the tasks it holds, read through the public verbs of
//! the two registries (the bundles registry's `fleet` and `get_count`, the root's
//! `reflect` of the project_lead's branch, the task registry's `list`) and shaped for the
//! dispatcher's `open` (the agents of the plan the page may chat with: [`read`], [`section`]) and the cadencer's digest rows
//! ([`tree_rows`], [`rows`], [`load`]). Pure functions over the replies; one read path.
//! No spawned agent is called and nothing here writes.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::dispatcher;
use crate::peers::Peers;

/// A worker is stalled only while holding a task and idle past the threshold.
/// The one definition: the cadencer's digest lines and the load counts use it.
pub fn is_stalled(holds_task: bool, idle_s: u64, stall_after_s: u64) -> bool {
    holds_task && idle_s >= stall_after_s
}

/// An agent's state from the statuses of the task_registry tasks it holds: `paused` when
/// any is paused; `active` when any is waiting, running or blocked, or when it holds no
/// task; `done` when all are done. A failed task with nothing live keeps it `active`.
fn state_of(statuses: &[&str]) -> &'static str {
    if statuses.contains(&"paused") {
        "paused"
    } else if !statuses.is_empty() && statuses.iter().all(|s| *s == "done") {
        "done"
    } else {
        "active"
    }
}

/// Who a registry task's `holder` names: an agent id, or `{subtree: id}` for a whole branch
/// (the agent itself or one of its `ancestors`).
pub fn holds(holder: &Value, id: &str, ancestors: &[String]) -> bool {
    match holder {
        Value::String(h) => h == id,
        other => other
            .get("subtree")
            .and_then(Value::as_str)
            .is_some_and(|r| r == id || ancestors.iter().any(|a| a == r)),
    }
}

/// Give `node` and everything below it a bundle and a state; push each descendant onto `flat`.
fn annotate(
    node: &Value,
    parent: Option<&str>,
    ancestors: &mut Vec<String>,
    bundles: &BTreeMap<String, String>,
    tasks: &[Value],
    flat: &mut Vec<Value>,
) -> Value {
    let id = node["id"].as_str().unwrap_or("").to_string();
    let held: Vec<&str> = tasks
        .iter()
        .filter(|t| holds(&t["holder"], &id, ancestors))
        .filter_map(|t| t["status"].as_str())
        .collect();
    let bundle = bundles.get(&id).cloned().unwrap_or_default();
    let state = state_of(&held);
    let mut out = node.clone();
    out["bundle"] = json!(bundle);
    out["state"] = json!(state);
    if let Some(p) = parent {
        flat.push(json!({"id": id, "bundle": bundle, "state": state, "parent": p}));
    }
    ancestors.push(id.clone());
    let kids: Vec<Value> = node["children"]
        .as_array()
        .map(|c| {
            c.iter()
                .map(|k| annotate(k, Some(&id), ancestors, bundles, tasks, flat))
                .collect()
        })
        .unwrap_or_default();
    ancestors.pop();
    out["children"] = json!(kids);
    out
}

/// The plan's section from the registries' replies: `lead` (the first project_lead of the
/// bundles registry's `fleet`), `fleet` (its `get_count`, passed through as it answers),
/// `tree` (the `reflect` node of the project_lead, each node with its bundle and state),
/// `agents` (the tree flat, the lead left out) and `tasks` (the task registry's `list`,
/// each as {id, source, holder, status}). An error reply leaves its part empty or null.
pub fn section(fleet: &Value, count: &Value, tree: &Value, tasks: &Value) -> Value {
    let mut bundles = BTreeMap::new();
    let mut lead: Option<String> = None;
    if let Some(m) = fleet.get("bundles").and_then(Value::as_object) {
        for (bundle, list) in m {
            for e in list.as_array().into_iter().flatten() {
                let id = e.as_str().or_else(|| e["id"].as_str());
                if let Some(id) = id {
                    bundles.insert(id.to_string(), bundle.clone());
                    if bundle == "project_lead" && lead.is_none() {
                        lead = Some(id.to_string());
                    }
                }
            }
        }
    }
    let rows: Vec<Value> = tasks
        .get("tasks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut agents = Vec::new();
    let tree = if tree.get("id").is_some() {
        annotate(tree, None, &mut Vec::new(), &bundles, &rows, &mut agents)
    } else {
        Value::Null
    };
    let tasks: Vec<Value> = rows
        .iter()
        .map(|t| {
            json!({"id": t["id"], "source": t["source"], "holder": t["holder"], "status": t["status"]})
        })
        .collect();
    json!({
        "lead": lead,
        "fleet": dispatcher::fleet_of(count),
        "tree": tree,
        "agents": agents,
        "tasks": tasks,
    })
}

/// The node `id` in a `reflect` reply's `tree` (a list of branches under `show_subtrees`,
/// or one tree).
fn find_node(tree: &Value, id: &str) -> Option<Value> {
    if tree.get("id").and_then(Value::as_str) == Some(id) {
        return Some(tree.clone());
    }
    let list = tree.as_array().or_else(|| tree["children"].as_array())?;
    list.iter().find_map(|n| find_node(n, id))
}

/// The four registry reads, through the public verbs only: the bundles registry's `fleet`
/// and `get_count`, the root's `reflect` of the project_lead's branch and the task
/// registry's `list`. A peer that errors leaves its part an error value (or `Null` for
/// the tree); nothing panics. Returns `(fleet, count, tree, tasks)`.
async fn reads(peers: &dyn Peers, bundles: &str) -> (Value, Value, Value, Value) {
    let fleet = peers.ask(bundles, json!({"type": "fleet"})).await;
    let count = peers.ask(bundles, json!({"type": "get_count"})).await;
    let lead = fleet["bundles"]["project_lead"]
        .as_array()
        .and_then(|l| l.first())
        .and_then(|e| e.as_str().or_else(|| e["id"].as_str()))
        .map(str::to_string);
    let mut tree = Value::Null;
    if let Some(pl) = &lead {
        let r = peers
            .ask(
                "kernel",
                json!({"type": "reflect", "show_subtrees": [pl], "tree": "all"}),
            )
            .await;
        tree = find_node(&r["tree"], pl).unwrap_or(Value::Null);
    }
    let tasks = peers
        .ask(crate::tree::TASKS_ID, json!({"type": "list"}))
        .await;
    (fleet, count, tree, tasks)
}

/// The plan read through the public verbs only ([`reads`]), as [`section`] shapes it. A
/// peer that errors leaves its part empty; nothing panics.
pub async fn read(peers: &dyn Peers, bundles: &str) -> Value {
    let (fleet, count, tree, tasks) = reads(peers, bundles).await;
    section(&fleet, &count, &tree, &tasks)
}

/// The newest `at` (seconds) a task shows: over its history entries and its report.
pub fn task_at(t: &Value) -> Option<f64> {
    t["history"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|h| h["at"].as_f64())
        .chain(t["report"]["at"].as_f64())
        .fold(None, |m: Option<f64>, a| Some(m.map_or(a, |m| m.max(a))))
}

/// The oldest `at` of a task's history: when it was first held.
fn task_since(t: &Value) -> Option<f64> {
    t["history"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|h| h["at"].as_f64())
        .fold(None, |m: Option<f64>, a| Some(m.map_or(a, |m| m.min(a))))
}

/// One row per agent of the spawned tree, in the shape `cadencer` reads (`id`, `parent`,
/// `has_ai`, `driver`, `state`, `task`, `idle_s`, `time_on_task_s`, `cost`, `cache_hit`,
/// `new_results`): the project_lead is the root (`parent` null), every other agent has
/// its spawner as parent. From the registries' replies and the clock in seconds.
/// An agent holding a running, waiting or blocked task (itself, or by `{subtree}` of an
/// ancestor) is `busy` with that task, and idles for `now` minus the newest `at` of the
/// task's history and report; one holding a paused task is `paused`; one whose tasks
/// are all done is `done`; the rest are `idle`. No tree, no rows.
pub fn rows(tree: &Value, tasks: &Value, now: u64) -> Vec<Value> {
    let rows: &[Value] = tasks
        .get("tasks")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let mut out = Vec::new();
    if tree.get("id").is_some() {
        rows_of(tree, None, &mut Vec::new(), rows, now as f64, &mut out);
    }
    out
}

fn rows_of(
    node: &Value,
    parent: Option<&str>,
    ancestors: &mut Vec<String>,
    tasks: &[Value],
    now: f64,
    out: &mut Vec<Value>,
) {
    let id = node["id"].as_str().unwrap_or("").to_string();
    let held: Vec<&Value> = tasks
        .iter()
        .filter(|t| holds(&t["holder"], &id, ancestors))
        .collect();
    let live: Vec<&Value> = held
        .iter()
        .copied()
        .filter(|t| {
            matches!(
                t["status"].as_str(),
                Some("running" | "waiting" | "blocked")
            )
        })
        .collect();
    let paused = held.iter().any(|t| t["status"] == "paused");
    // The task the agent is working on: a running one first.
    let current = live
        .iter()
        .find(|t| t["status"] == "running")
        .or_else(|| live.first())
        .copied();
    let statuses: Vec<&str> = held.iter().filter_map(|t| t["status"].as_str()).collect();
    let state = if !live.is_empty() {
        "busy"
    } else if paused {
        "paused"
    } else if !statuses.is_empty() && statuses.iter().all(|s| *s == "done") {
        "done"
    } else {
        "idle"
    };
    let secs = |from: Option<f64>| from.map_or(0, |a| (now - a).max(0.0) as u64);
    out.push(json!({
        "id": id,
        "parent": parent,
        "has_ai": true,
        "driver": "",
        "state": state,
        "task": current.and_then(|t| t["id"].as_str()).unwrap_or(""),
        "idle_s": secs(current.and_then(task_at)),
        "time_on_task_s": secs(current.and_then(task_since)),
        "cost": 0,
        "cache_hit": 0,
        "new_results": 0,
    }));
    ancestors.push(id.clone());
    for k in node["children"].as_array().into_iter().flatten() {
        rows_of(k, Some(&id), ancestors, tasks, now, out);
    }
    ancestors.pop();
}

/// The `load` object: the agents below the project_lead busy, idle and stalled, and the
/// registry's tasks by status, with `queued` the waiting ones, so the quiet heartbeat
/// can fire.
pub fn load(rows: &[Value], tasks: &Value, stall_after_s: u64) -> Value {
    let workers: Vec<&Value> = rows.iter().filter(|r| !r["parent"].is_null()).collect();
    let busy = workers.iter().filter(|r| r["state"] == "busy").count();
    let idle = workers.iter().filter(|r| r["state"] == "idle").count();
    let stalled = workers
        .iter()
        .filter(|r| {
            is_stalled(
                !r["task"].as_str().unwrap_or("").is_empty(),
                r["idle_s"].as_u64().unwrap_or(0),
                stall_after_s,
            )
        })
        .count();
    let count = |st: &str| {
        tasks["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| t["status"] == st)
            .count()
    };
    json!({
        "workers": {"busy": busy, "idle": idle, "stalled": stalled},
        "tasks": {
            "held": count("running"),
            "blocked": count("blocked"),
            "done": count("done"),
            "queued": count("waiting"),
        },
    })
}

/// The spawned tree for the cadencer: the rows ([`rows`]) and the load ([`load`]), read
/// through [`reads`] (public verbs only; no spawned agent is called). A peer that errors
/// gives no rows.
pub async fn tree_rows(
    peers: &dyn Peers,
    bundles: &str,
    now: u64,
    stall_after_s: u64,
) -> (Vec<Value>, Value) {
    let (_, _, tree, tasks) = reads(peers, bundles).await;
    let rows = rows(&tree, &tasks, now);
    let load = load(&rows, &tasks, stall_after_s);
    (rows, load)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A scripted bundles registry, kernel reflect and task registry; records every call.
    struct BundlesPeers {
        fleet: Value,
        count: Value,
        reflect: Value,
        tasks: Value,
        calls: Mutex<Vec<(String, Value)>>,
    }

    #[async_trait::async_trait]
    impl Peers for BundlesPeers {
        async fn ask(&self, to: &str, p: Value) -> Value {
            self.calls.lock().unwrap().push((to.to_string(), p.clone()));
            match (to, p["type"].as_str().unwrap_or("")) {
                ("bundles", "fleet") => self.fleet.clone(),
                ("bundles", "get_count") => self.count.clone(),
                ("kernel", "reflect") => self.reflect.clone(),
                ("tasks", "list") => self.tasks.clone(),
                _ => json!({"error": "unexpected"}),
            }
        }
    }

    fn bundles_peers() -> BundlesPeers {
        BundlesPeers {
            fleet: json!({"bundles": {"project_lead": [{"id": "pl"}], "worker": [{"id": "w1"}, {"id": "w2"}]}}),
            count: json!({"running": 3}),
            reflect: json!({"tree": [{"id": "pl", "children": [{"id": "w1", "children": []}, {"id": "w2", "children": []}]}]}),
            tasks: json!({"tasks": [
                {"id": "t1", "source": "issue 7", "holder": "w1", "status": "running", "history": []}
            ]}),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn down_peers() -> BundlesPeers {
        let e = json!({"error": "down"});
        BundlesPeers {
            fleet: e.clone(),
            count: e.clone(),
            reflect: e.clone(),
            tasks: e,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn fleet_pl() -> Value {
        json!({"bundles": {"project_lead": [{"id": "pl"}], "worker": [{"id": "w1"}, {"id": "w2"}]}})
    }

    fn tree_pl() -> Value {
        json!({"id": "pl", "children": [
            {"id": "w1", "children": []}, {"id": "w2", "children": []}
        ]})
    }

    fn state_of_agent(v: &Value, id: &str) -> Value {
        v["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == id)
            .unwrap_or_else(|| panic!("no agent {id}"))["state"]
            .clone()
    }

    #[test]
    fn section_lists_the_tree_below_the_project_lead_with_bundles() {
        let v = section(&fleet_pl(), &json!({}), &tree_pl(), &json!({"tasks": []}));
        assert_eq!(v["lead"], "pl");
        assert_eq!(v["tree"]["id"], "pl");
        assert_eq!(v["tree"]["bundle"], "project_lead");
        assert_eq!(v["tree"]["children"][0]["bundle"], "worker");
        let agents = v["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 2, "the lead is the root, not listed again");
        for id in ["w1", "w2"] {
            let a = agents.iter().find(|a| a["id"] == id).unwrap();
            assert_eq!(a["bundle"], "worker");
            assert_eq!(a["parent"], "pl");
        }
    }

    #[test]
    fn section_agent_state_comes_from_its_tasks() {
        let fleet = json!({"bundles": {"project_lead": [{"id": "pl"}], "worker": [
            {"id": "w1"}, {"id": "w2"}, {"id": "w3"}, {"id": "w4"}]}});
        let tree = json!({"id": "pl", "children": [
            {"id": "w1", "children": []}, {"id": "w2", "children": []},
            {"id": "w3", "children": []}, {"id": "w4", "children": []}]});
        let tasks = json!({"tasks": [
            {"id": "a", "source": "s", "holder": "w1", "status": "running"},
            {"id": "b", "source": "s", "holder": "w2", "status": "paused"},
            {"id": "c", "source": "s", "holder": "w3", "status": "done"}
        ]});
        let v = section(&fleet, &json!({}), &tree, &tasks);
        assert_eq!(state_of_agent(&v, "w1"), "active");
        assert_eq!(state_of_agent(&v, "w2"), "paused");
        assert_eq!(state_of_agent(&v, "w3"), "done");
        assert_eq!(state_of_agent(&v, "w4"), "active");
    }

    #[test]
    fn section_subtree_holder_covers_descendants() {
        let tree = json!({"id": "pl", "children": [
            {"id": "sub", "children": [{"id": "kid", "children": []}]},
            {"id": "other", "children": []}]});
        let tasks = json!({"tasks": [
            {"id": "a", "source": "s", "holder": {"subtree": "sub"}, "status": "paused"}
        ]});
        let v = section(&json!({}), &json!({}), &tree, &tasks);
        assert_eq!(state_of_agent(&v, "sub"), "paused");
        assert_eq!(state_of_agent(&v, "kid"), "paused");
        assert_eq!(state_of_agent(&v, "other"), "active");
    }

    #[test]
    fn section_lists_tasks_with_source_holder_status() {
        let tasks = json!({"tasks": [
            {"id": "t1", "source": "issue 7", "holder": "w1", "status": "running", "history": [1]},
            {"id": "t2", "source": "chat", "holder": {"subtree": "sub"}, "status": "waiting", "history": []}
        ]});
        let v = section(&fleet_pl(), &json!({}), &tree_pl(), &tasks);
        assert_eq!(
            v["tasks"],
            json!([
                {"id": "t1", "source": "issue 7", "holder": "w1", "status": "running"},
                {"id": "t2", "source": "chat", "holder": {"subtree": "sub"}, "status": "waiting"}
            ])
        );
    }

    #[test]
    fn section_fleet_is_the_registrys_count_as_is() {
        let v = section(&json!({}), &json!({"running": 3}), &Value::Null, &json!({}));
        assert_eq!(v["fleet"], json!({"running": 3}));
        let v = section(
            &json!({}),
            &json!({"error": "no"}),
            &Value::Null,
            &json!({}),
        );
        assert!(v["fleet"].is_null());
    }

    #[tokio::test]
    async fn read_goes_through_the_public_verbs_only() {
        let peers = bundles_peers();
        let v = read(&peers, "bundles").await;
        let calls: Vec<(String, String)> = peers
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|(to, p)| (to.clone(), p["type"].as_str().unwrap_or("").to_string()))
            .collect();
        let want = [
            ("bundles", "fleet"),
            ("bundles", "get_count"),
            ("kernel", "reflect"),
            ("tasks", "list"),
        ];
        assert_eq!(
            calls,
            want.iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            peers.calls.lock().unwrap()[2].1["show_subtrees"],
            json!(["pl"])
        );
        assert_eq!(v["tree"]["id"], "pl");
        assert_eq!(v["fleet"], json!({"running": 3}));
        assert_eq!(v["tasks"][0]["id"], "t1");
    }

    #[tokio::test]
    async fn read_survives_errors() {
        let v = read(&down_peers(), "bundles").await;
        assert!(v["tree"].is_null());
        assert!(v["lead"].is_null());
        assert_eq!(v["agents"], json!([]));
        assert_eq!(v["tasks"], json!([]));
    }

    fn row_of<'a>(rows: &'a [Value], id: &str) -> &'a Value {
        rows.iter()
            .find(|r| r["id"] == id)
            .unwrap_or_else(|| panic!("no row {id}"))
    }

    fn history(at: f64) -> Value {
        json!([{"from": null, "to": "w1", "by": "pl", "at": at}])
    }

    #[test]
    fn rows_have_the_feed_shape() {
        let tree = json!({"id": "pl", "children": [
            {"id": "sub", "children": [{"id": "w1", "children": []}]},
            {"id": "w2", "children": []}]});
        let tasks = json!({"tasks": [
            {"id": "t1", "source": "s", "holder": "w1", "status": "running",
             "history": history(900.0), "report": {"from": "w1", "at": 940.0}},
            {"id": "t2", "source": "s", "holder": "w2", "status": "waiting", "history": history(990.0)}
        ]});
        let rows = rows(&tree, &tasks, 1000);
        assert_eq!(rows.len(), 4);
        let pl = row_of(&rows, "pl");
        assert!(pl["parent"].is_null());
        assert_eq!(pl["has_ai"], true);
        assert_eq!(pl["driver"], "");
        assert_eq!(row_of(&rows, "sub")["parent"], "pl");
        let w1 = row_of(&rows, "w1");
        assert_eq!(w1["parent"], "sub");
        assert_eq!(w1["state"], "busy");
        assert_eq!(w1["task"], "t1");
        assert_eq!(w1["idle_s"], 60, "now minus the newest history/report at");
        assert_eq!(w1["time_on_task_s"], 100, "now minus the oldest history at");
        assert_eq!(row_of(&rows, "w2")["idle_s"], 10);
        for k in ["cost", "cache_hit", "new_results"] {
            assert_eq!(w1[k], 0, "{k}");
        }
    }

    #[test]
    fn rows_subtree_holder_marks_descendants_busy() {
        let tree = json!({"id": "pl", "children": [
            {"id": "sub", "children": [{"id": "kid", "children": []}]},
            {"id": "other", "children": []}]});
        let tasks = json!({"tasks": [
            {"id": "a", "source": "s", "holder": {"subtree": "sub"}, "status": "running", "history": history(500.0)}
        ]});
        let rows = rows(&tree, &tasks, 600);
        assert_eq!(row_of(&rows, "sub")["state"], "busy");
        assert_eq!(row_of(&rows, "kid")["state"], "busy");
        assert_eq!(row_of(&rows, "kid")["task"], "a");
        assert_eq!(row_of(&rows, "other")["state"], "idle");
        assert_eq!(row_of(&rows, "pl")["state"], "idle");
    }

    #[test]
    fn rows_idle_agent_without_task_is_idle_not_stalled() {
        let tree = json!({"id": "pl", "children": [
            {"id": "w1", "children": []}, {"id": "w2", "children": []}]});
        let tasks = json!({"tasks": [
            {"id": "a", "source": "s", "holder": "w2", "status": "done", "history": history(1.0)}
        ]});
        let rows = rows(&tree, &tasks, 100_000);
        let w1 = row_of(&rows, "w1");
        assert_eq!(
            (w1["state"].as_str(), w1["task"].as_str()),
            (Some("idle"), Some(""))
        );
        assert_eq!(w1["idle_s"], 0);
        assert_eq!(row_of(&rows, "w2")["state"], "done");
        assert_eq!(row_of(&rows, "w2")["task"], "");
        let load = load(&rows, &tasks, 300);
        assert_eq!(load["workers"]["stalled"], 0);
        assert_eq!(load["workers"]["idle"], 1);
    }

    #[test]
    fn load_counts_queued_and_stalled() {
        let tree = json!({"id": "pl", "children": [{"id": "w1", "children": []}]});
        let tasks = json!({"tasks": [
            {"id": "a", "source": "s", "holder": "w1", "status": "running", "history": history(0.0)},
            {"id": "b", "source": "s", "holder": null, "status": "waiting", "history": []},
            {"id": "c", "source": "s", "holder": null, "status": "waiting", "history": []}
        ]});
        let rows = rows(&tree, &tasks, 1000);
        let load = load(&rows, &tasks, 300);
        assert_eq!(load["tasks"]["queued"], 2);
        assert_eq!(load["tasks"]["held"], 1);
        assert_eq!(load["workers"]["busy"], 1);
        assert_eq!(load["workers"]["stalled"], 1);
    }

    #[test]
    fn is_stalled_needs_a_task_and_the_idle_time() {
        assert!(is_stalled(true, 300, 300));
        assert!(!is_stalled(true, 299, 300));
        assert!(!is_stalled(false, 10_000, 300));
    }

    #[tokio::test]
    async fn tree_rows_reads_the_public_verbs_only() {
        let peers = bundles_peers();
        let (rows, load) = tree_rows(&peers, "bundles", 1000, 300).await;
        assert_eq!(rows.len(), 3);
        assert_eq!(load["workers"]["busy"], 1);
        let calls: Vec<String> = peers
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|(to, p)| format!("{to}:{}", p["type"].as_str().unwrap_or("")))
            .collect();
        assert_eq!(
            calls,
            [
                "bundles:fleet",
                "bundles:get_count",
                "kernel:reflect",
                "tasks:list"
            ]
        );
    }

    #[tokio::test]
    async fn tree_rows_survives_peer_errors() {
        let (rows, load) = tree_rows(&down_peers(), "bundles", 1000, 300).await;
        assert!(rows.is_empty());
        assert_eq!(load["tasks"]["queued"], 0);
    }
}
