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
    /// The number of the pull request of its task, when the task has a link.
    pub pr: Option<u64>,
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

/// The pull request number in a task's `pr` link: the digits after `/pull/`, or
/// after a trailing `#` (`owner/repo#7`). None for anything else.
pub fn pr_number(link: &str) -> Option<u64> {
    let link = link.trim().trim_end_matches('/');
    let digits = |rest: &str| -> Option<u64> {
        let d: String = rest.chars().take_while(char::is_ascii_digit).collect();
        d.parse().ok()
    };
    if let Some(i) = link.rfind("/pull/") {
        return digits(&link[i + 6..]);
    }
    let i = link.rfind('#')?;
    let rest = &link[i + 1..];
    (!rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())).then(|| digits(rest))?
}

/// Build the row for one worker from the board's snapshot.
pub fn row_of(worker: &str, board: &Value, now: u64, limit: u64) -> Row {
    let agent = &board["agents"][worker];
    let cost = agent["cost"].as_f64().unwrap_or(0.0);
    // The worker's current task: the first in-work task it claims.
    let mut task = None;
    let mut status = "pending";
    let mut updated = 0u64;
    let mut pr = None;
    if let Some(tasks) = board["tasks"]["in_work"].as_array() {
        for t in tasks {
            let claimed = t["claimers"]
                .as_array()
                .is_some_and(|c| c.iter().any(|x| x.as_str() == Some(worker)));
            if claimed {
                task = t["id"].as_str().map(str::to_string);
                status = "in_work";
                updated = t["updated"].as_u64().unwrap_or(0);
                pr = t["pr"].as_str().and_then(pr_number);
                break;
            }
        }
    }
    if pr.is_none() && status != "in_work" {
        // No task in work: the newest done task it claimed that has a pull request.
        let mut newest = 0u64;
        for t in board["tasks"]["done"].as_array().into_iter().flatten() {
            let mine = t["claimers"]
                .as_array()
                .is_some_and(|c| c.iter().any(|x| x.as_str() == Some(worker)));
            let n = t["pr"].as_str().and_then(pr_number);
            let at = t["updated"].as_u64().unwrap_or(0);
            if mine && n.is_some() && (pr.is_none() || at >= newest) {
                pr = n;
                newest = at;
            }
        }
    }
    let minutes = (updated > 0).then(|| now.saturating_sub(updated) / 60);
    Row {
        id: worker.into(),
        state: state_label(status, minutes, limit),
        task,
        minutes,
        pr,
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
        "minutes": r.minutes, "pr": r.pr,
        "stalled": r.state.starts_with("stalled"),
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
/// {tasks, done_last_hour} when the board gave one, plus the cadencer's
/// `digest_bytes_per_hour` and `driver_tokens_per_hour` when `load` carries them,
/// and `waiting` {prs, stalled}: the PRs waiting for approval and the stalled
/// workers, the figures that need the person. (The bundle adds `fleet` after this.)
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
        let mut team_load = json!({
            "tasks": load["tasks"],
            "done_last_hour": load["done_last_hour"],
        });
        for k in ["digest_bytes_per_hour", "driver_tokens_per_hour"] {
            if load.get(k).is_some_and(|v| !v.is_null()) {
                team_load[k] = load[k].clone();
            }
        }
        out["load"] = team_load;
        out["waiting"] = json!({
            "prs": load["tasks"]["in_review"].as_u64().unwrap_or(0),
            "stalled": load["workers"]["stalled"].as_u64().unwrap_or(0),
        });
    }
    out
}

/// The sidebar's `fleet` {running, max} from a registry's `get_count` reply: none when the
/// reply is an error or lacks a count. `max` is null when the registry has no cap.
pub fn fleet_of(count: &Value) -> Option<Value> {
    if count.get("error").is_some() {
        return None;
    }
    let running = count.get("running")?.as_u64()?;
    Some(json!({"running": running, "max": count.get("max").cloned().unwrap_or(Value::Null)}))
}

/// The full-screen chat for one agent (a worker or a subagent). The chat targets
/// that agent alone; Back returns to the sidebar. The page streams the chat
/// itself, over the chat verbs granted by `tree::grant_chat`: it loads the history,
/// sends, stops and watches the parts. This reply only names the target and the
/// cell that is drawn.
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

/// The actions on a worker or a group that the dispatcher carries out itself:
/// hand a worker a task, add a worker to a group, remove a worker.
pub const WORKER_ACTIONS: [&str; 3] = ["hand_task", "add_worker", "remove_worker"];

/// `stop` and `remove_worker` need an explicit confirm; without it the answer is a question.
pub fn control_reply(node: &str, action: &str, confirm: bool) -> Value {
    if !CONTROL_ACTIONS.contains(&action) && !WORKER_ACTIONS.contains(&action) {
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
    if action == "remove_worker" && !confirm {
        return json!({
            "question": format!("Remove {node}? Send again with confirm: true."),
            "sent": false,
        });
    }
    json!({"sent": true, "node": node, "action": action})
}

/// The groups with `id` added to `group`. An unknown group or an id already in
/// any group is an error.
pub fn groups_with(
    groups: &BTreeMap<String, Vec<String>>,
    group: &str,
    id: &str,
) -> Result<BTreeMap<String, Vec<String>>, String> {
    if !groups.contains_key(group) {
        return Err(format!("team_dispatcher: unknown group '{group}'"));
    }
    if groups.values().any(|m| m.iter().any(|x| x == id)) {
        return Err(format!("team_dispatcher: '{id}' is already in a group"));
    }
    let mut out = groups.clone();
    out.entry(group.to_string())
        .or_default()
        .push(id.to_string());
    Ok(out)
}

/// The groups with `id` removed from every one.
pub fn groups_without(
    groups: &BTreeMap<String, Vec<String>>,
    id: &str,
) -> BTreeMap<String, Vec<String>> {
    groups
        .iter()
        .map(|(g, m)| (g.clone(), m.iter().filter(|x| *x != id).cloned().collect()))
        .collect()
}

/// The board's `team_groups` value with the members of each group replaced by
/// `groups`; each group's driver is kept.
fn team_groups_with(current: &Value, groups: &BTreeMap<String, Vec<String>>) -> Value {
    let mut out = current.as_object().cloned().unwrap_or_default();
    for (g, members) in groups {
        let mut entry = out.get(g).cloned().unwrap_or_else(|| json!({"driver": ""}));
        entry["members"] = json!(members);
        out.insert(g.clone(), entry);
    }
    Value::Object(out)
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

/// The reply to a `control` that was relayed to a node: its refusal or no reply
/// is `sent: false` with the error; otherwise the node's reply rides along.
fn relayed(reply: Value, node: &str, action: &str, peer: Value) -> Value {
    let peer_error = if peer.is_null() {
        Some(json!(format!("team_dispatcher: no reply from '{node}'")))
    } else {
        peer.get("error").filter(|e| !e.is_null()).cloned()
    };
    match peer_error {
        Some(error) => json!({"sent": false, "node": node, "action": action, "error": error}),
        None => {
            let mut out = reply;
            out["reply"] = peer;
            out
        }
    }
}

fn refused(node: &str, action: &str, error: String) -> Value {
    json!({"sent": false, "node": node, "action": action, "error": error})
}

/// The error text of a kernel reply, if it is a refusal.
fn error_of(reply: &Value) -> Option<String> {
    if reply.is_null() {
        return Some("no reply".into());
    }
    reply.get("error").filter(|e| !e.is_null()).map(|e| {
        e.as_str()
            .map(str::to_string)
            .unwrap_or_else(|| e.to_string())
    })
}

/// Write the new membership to the dispatcher's own record and to the board's
/// `team_groups`, through the kernel's `update_agent`. The board is skipped when
/// it is not there.
/// Where the groups are kept: the dispatcher's own record (`me`), the board's
/// `team_groups` (when the board is live), and the groups as they stand.
struct Membership<'a> {
    me: &'a str,
    board: &'a str,
    board_groups: Option<&'a Value>,
    groups: &'a BTreeMap<String, Vec<String>>,
}

async fn save_groups(
    peers: &dyn Peers,
    at: &Membership<'_>,
    groups: &BTreeMap<String, Vec<String>>,
) -> Result<(), String> {
    let (me, board, board_groups) = (at.me, at.board, at.board_groups);
    let root = fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
    let r = peers
        .ask(
            root,
            json!({"type": "update_agent", "id": me, "groups": groups}),
        )
        .await;
    if let Some(e) = error_of(&r) {
        return Err(format!("could not update the groups: {e}"));
    }
    let Some(current) = board_groups else {
        return Ok(());
    };
    let tg = team_groups_with(current, groups);
    let r = peers
        .ask(
            root,
            json!({"type": "update_agent", "id": board, "team_groups": tg}),
        )
        .await;
    match error_of(&r) {
        Some(e) => Err(format!("could not update the board's groups: {e}")),
        None => Ok(()),
    }
}

/// `add_worker {node: group, id, template}`: create the worker from the template of
/// `team.yaml` (in the working directory), point its memory at the team's one store
/// agent (and let it call that store), and list it. No memory agent is made.
async fn add_worker(
    kernel: &Arc<fantastic_kernel::Kernel>,
    peers: &dyn Peers,
    at: &Membership<'_>,
    group: &str,
    payload: &Value,
    reply: Value,
) -> Value {
    let action = "add_worker";
    let id = payload["id"].as_str().unwrap_or("").trim().to_string();
    let template = payload["template"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    if id.is_empty() || template.is_empty() {
        return refused(
            group,
            action,
            "team_dispatcher: add_worker needs id and template".into(),
        );
    }
    let new_groups = match groups_with(at.groups, group, &id) {
        Ok(g) => g,
        Err(e) => return refused(group, action, e),
    };
    let cfg = match crate::config::load(std::path::Path::new(crate::config::FILE_NAME)) {
        Ok(c) => c,
        Err(e) => return refused(group, action, format!("team_dispatcher: team.yaml: {e}")),
    };
    let Some(tpl) = cfg.templates.get(&template) else {
        let names: Vec<&str> = cfg.templates.keys().map(String::as_str).collect();
        return refused(
            group,
            action,
            format!(
                "team_dispatcher: no template '{template}' in team.yaml (have: {})",
                names.join(", ")
            ),
        );
    };
    let root = fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
    let store = cfg.store.id.as_str();
    let rec = crate::config::agent_record(tpl, &id, store);
    if let Some(e) = error_of(&peers.ask(root, rec).await) {
        return refused(group, action, format!("could not create '{id}': {e}"));
    }
    if let Err(e) = crate::tree::grant_store(kernel, store, &id) {
        // Only the worker is undone: the store and its rows are the team's.
        let _ = peers
            .ask(root, json!({"type": "delete_agent", "id": id}))
            .await;
        return refused(group, action, format!("could not create '{id}': {e}"));
    }
    if let Err(e) = save_groups(peers, at, &new_groups).await {
        return refused(group, action, e);
    }
    let mut out = reply;
    out["id"] = json!(id);
    out
}

/// `remove_worker {node: worker}`: delete the worker and drop it from its group. Its
/// rows stay in the team's one store agent.
async fn remove_worker(
    peers: &dyn Peers,
    at: &Membership<'_>,
    worker: &str,
    reply: Value,
) -> Value {
    let action = "remove_worker";
    let root = fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
    let r = peers
        .ask(root, json!({"type": "delete_agent", "id": worker}))
        .await;
    if let Some(e) = error_of(&r) {
        return refused(worker, action, format!("could not remove '{worker}': {e}"));
    }
    let left = groups_without(at.groups, worker);
    if let Err(e) = save_groups(peers, at, &left).await {
        return refused(worker, action, e);
    }
    reply
}

/// The dispatcher bundle: `sidebar`, `tree`, `open`, `control`.
#[derive(Default)]
pub struct DispatcherBundle;

#[async_trait::async_trait]
impl fantastic_kernel::bundle::Bundle for DispatcherBundle {
    fn name(&self) -> &str {
        HANDLER_MODULE
    }

    fn verb_docs(&self) -> Value {
        json!({
            "sidebar": "sidebar {expanded?: [worker ids]} → per group one flat list: each worker, its subagents expander, and its subagents when expanded; a group with no AI driver also gets its raw load counts; with a roles registry the reply also has fleet {running, max}, the registry's get_count",
            "tree": "tree {node?} → the tree of the team or a node, rows with state/task/minutes/cost",
            "open": "open {node} → the view to open for that node (worker or subagent → full-screen chat; group/team → a dispatcher)",
            "control": "control {node, action: start|pause|stop|hand_task|add_worker|remove_worker, confirm?, task?, id?, template?} → {sent, error?} (or a question for stop and remove_worker without confirm); the node's refusal or no reply is sent: false with its error. hand_task {node: worker, task} sends the worker `task <task>`. add_worker {node: group, id, template} makes the worker from the team.yaml template, its memory in the team's one store agent, in the working directory and lists it in the group (the dispatcher may send, stop, start and pause it at once; its chat grant follows at the next start; team.yaml is not rewritten). remove_worker {node: worker, confirm} deletes the worker (its rows stay in the store) and drops it from its group",
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
        // The board's own `team_groups`, kept in step when a worker is added or removed.
        let board_groups = kernel
            .agents
            .get(&fantastic_kernel::AgentId::from(board.as_str()))
            .map(|e| {
                e.meta
                    .read()
                    .expect("meta")
                    .get("team_groups")
                    .cloned()
                    .unwrap_or(Value::Null)
            });
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
                let mut load = reply.get("load").cloned().unwrap_or(Value::Null);
                // The cadencer's load holds the board's counts plus the cost per hour.
                let cad = peers
                    .ask(crate::tree::CADENCER_ID, json!({"type": "status"}))
                    .await;
                if cad.get("error").is_none() && cad["load"].is_object() {
                    load = cad["load"].clone();
                }
                let mut out = sidebar(&team, &groups, &rows, &subs, &expanded_of(payload), &load);
                // The roles registry's count (agents running, and the cap), when the
                // team has a registry and it answers.
                if let Some(roles) = meta.get("roles").and_then(Value::as_str) {
                    let c = peers.ask(roles, json!({"type": "get_count"})).await;
                    if let Some(f) = fleet_of(&c) {
                        out["fleet"] = f;
                    }
                }
                out
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
                match action {
                    "hand_task" => {
                        let task = payload["task"].as_str().unwrap_or("").trim().to_string();
                        if task.is_empty() {
                            return Ok(Some(json!({
                                "error": "team_dispatcher: hand_task needs a task",
                                "sent": false,
                            })));
                        }
                        let peer = peers
                            .ask(
                                &node,
                                json!({"type": "send", "text": format!("task {task}")}),
                            )
                            .await;
                        relayed(reply, &node, action, peer)
                    }
                    "add_worker" => {
                        let at = Membership {
                            me: agent_id.as_str(),
                            board: &board,
                            board_groups: board_groups.as_ref(),
                            groups: &groups,
                        };
                        let out = add_worker(kernel, &peers, &at, &node, payload, reply).await;
                        // The new worker may be steered by the dispatcher at once,
                        // like the workers granted at the start (`grant_control`).
                        if out.get("error").is_none() {
                            if let Some(id) = out["id"].as_str() {
                                crate::tree::grant_control(kernel, &[id.to_string()]);
                            }
                        }
                        out
                    }
                    "remove_worker" => {
                        let at = Membership {
                            me: agent_id.as_str(),
                            board: &board,
                            board_groups: board_groups.as_ref(),
                            groups: &groups,
                        };
                        remove_worker(&peers, &at, &node, reply).await
                    }
                    _ => {
                        // The node's own answer decides: a refusal or no reply is not "sent".
                        let peer = peers.ask(&node, json!({"type": action})).await;
                        relayed(reply, &node, action, peer)
                    }
                }
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
    }

    #[test]
    fn row_shows_minutes_on_the_task() {
        let b = json!({
            "tasks": {"in_work": [{"id": "r#1", "claimers": ["w1"], "updated": 600}], "done": [], "pending": [], "failed": []},
            "agents": {}
        });
        let r = row_of("w1", &b, 1200, 30);
        assert_eq!(r.minutes, Some(10));
        // A worker with no task has no minutes.
        assert_eq!(row_of("w2", &b, 1200, 30).minutes, None);
    }

    #[test]
    fn row_carries_the_pr_number_of_its_done_task() {
        let b = json!({
            "tasks": {"in_work": [], "pending": [], "failed": [], "done": [
                {"id": "r#1", "claimers": ["w1"], "updated": 100, "pr": "https://github.com/o/r/pull/7"},
                {"id": "r#2", "claimers": ["w1"], "updated": 300, "pr": "https://github.com/o/r/pull/9"},
                {"id": "r#3", "claimers": ["w1"], "updated": 900},
                {"id": "r#4", "claimers": ["w3"], "updated": 400, "pr": "https://github.com/o/r/pull/11"}
            ]},
            "agents": {}
        });
        // The most recently updated done task that has a PR.
        assert_eq!(row_of("w1", &b, 1000, 30).pr, Some(9));
        assert_eq!(row_of("w2", &b, 1000, 30).pr, None);
        // A task in hand with a PR shows that PR.
        let held = json!({
            "tasks": {"in_work": [{"id": "r#5", "claimers": ["w2"], "updated": 10, "pr": "o/r#12"}], "done": [], "pending": [], "failed": []},
            "agents": {}
        });
        assert_eq!(row_of("w2", &held, 100, 30).pr, Some(12));
    }

    #[test]
    fn pr_number_reads_url_and_hash_forms() {
        assert_eq!(pr_number("https://github.com/o/r/pull/9"), Some(9));
        assert_eq!(pr_number("https://github.com/o/r/pull/9/"), Some(9));
        assert_eq!(pr_number("o/r#42"), Some(42));
        assert_eq!(pr_number("#7"), Some(7));
        assert_eq!(pr_number("https://github.com/o/r/pull/"), None);
        assert_eq!(pr_number("not a pr"), None);
        assert_eq!(pr_number(""), None);
    }

    #[test]
    fn row_of_carries_minutes_into_sidebar_item() {
        let b = json!({
            "tasks": {"in_work": [{"id": "r#1", "claimers": ["w1"], "updated": 1000}], "done": []},
            "agents": {"w1": {"cost": 0.0}}
        });
        let r = row_of("w1", &b, 1120, 30);
        assert_eq!(r.minutes, Some(2));
        let items = worker_items(&r, &BTreeMap::new(), &BTreeSet::new());
        assert_eq!(items[0]["minutes"], json!(2));
        let idle = row_of("w2", &b, 1120, 30);
        let items = worker_items(&idle, &BTreeMap::new(), &BTreeSet::new());
        assert!(items[0]["minutes"].is_null());
        assert!(items[0]["pr"].is_null());
    }

    #[test]
    fn row_of_exposes_pr_number_from_claimed_task_link() {
        let b = json!({
            "tasks": {
                "in_work": [{"id": "r#1", "claimers": ["w1"], "updated": 5, "pr": "https://github.com/o/r/pull/12"}],
                "done": [
                    {"id": "r#2", "claimers": ["w2"], "updated": 1, "pr": "https://github.com/o/r/pull/3"},
                    {"id": "r#3", "claimers": ["w2"], "updated": 9, "pr": "https://github.com/o/r/pull/8"},
                    {"id": "r#4", "claimers": ["w2"], "updated": 4, "pr": "https://github.com/o/r/pull/5"},
                    {"id": "r#5", "claimers": ["w4"], "updated": 20, "pr": "https://github.com/o/r/pull/99"},
                    {"id": "r#6", "claimers": ["w3"], "updated": 20}
                ]
            },
            "agents": {}
        });
        assert_eq!(row_of("w1", &b, 100, 30).pr, Some(12));
        assert_eq!(row_of("w2", &b, 100, 30).pr, Some(8));
        assert_eq!(row_of("w3", &b, 100, 30).pr, None);
        let nolink = json!({"tasks": {"in_work": [{"id": "r#1", "claimers": ["w1"], "updated": 5}], "done": []}});
        assert_eq!(row_of("w1", &nolink, 100, 30).pr, None);
    }

    #[test]
    fn pr_number_parses_links_and_refs() {
        assert_eq!(pr_number("https://github.com/o/r/pull/9"), Some(9));
        assert_eq!(pr_number("https://github.com/o/r/pull/9/"), Some(9));
        assert_eq!(pr_number("o/r#7"), Some(7));
        assert_eq!(pr_number("junk"), None);
        assert_eq!(pr_number(""), None);
        assert_eq!(pr_number("o/r#"), None);
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
    fn html_shows_pack_pass_rate_time_cost_and_verdict() {
        assert!(HTML.contains("askNode('pack'"));
        assert!(HTML.contains("type: 'install'"));
        assert!(HTML.contains("evals.pass_rate"));
        assert!(HTML.contains("ev.pass_rate"));
        assert!(HTML.contains("ev.total_time_s"));
        assert!(HTML.contains("ev.total_cost"));
        assert!(HTML.contains("'n/a'"));
        assert!(HTML.contains("enable.allowed"));
        assert!(HTML.contains("enable.reason"));
        assert!(HTML.contains("Pack install"));
        assert!(HTML.contains("textContent"));
        assert!(!HTML.contains("innerHTML"));
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
    fn html_draws_team_load_footer() {
        // The team load the sidebar reply carries is drawn as a footer row.
        assert!(
            HTML.contains("reply.load"),
            "the page never reads reply.load"
        );
        // The row is created with the team-load class and the footer text.
        assert!(HTML.contains("el('div', 'team-load'"));
        assert!(HTML.contains(" · done last hour ' + num(reply.load.done_last_hour)"));
        for key in ["held", "in_review", "done", "blocked", "queued"] {
            assert!(
                HTML.contains(&format!("tasks.{key}")),
                "no tasks.{key} in the footer"
            );
        }
    }

    #[test]
    fn html_team_load_footer_is_text_only_and_optional() {
        // Without a load from the board, no footer row is drawn.
        assert!(HTML.contains("if (reply && reply.load"));
        // The footer is text only, like the rest of the page.
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
            json!({"type": "control", "node": "probe", "action": "frobnicate"}),
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
    /// brace at column zero. Line endings are normalised first: a Windows
    /// checkout has CRLF in the embedded page.
    fn js_fn(name: &str) -> String {
        let html = HTML.replace("\r\n", "\n");
        let start = html
            .find(&format!("function {name}("))
            .unwrap_or_else(|| panic!("no function {name}"));
        let end = html[start..].find("\n}\n").expect("closing brace") + start;
        html[start..end].to_string()
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

    #[test]
    fn dispatcher_page_streams_chat() {
        // The chat runs over the chat verbs and the part events of the target agent.
        for needle in [
            "type: 'history'",
            "type: 'send'",
            "type: 'stop'",
            "fantastic.watch(",
            "'part_start'",
            "'part_delta'",
            "'part_end'",
            "'part_abort'",
            "'done'",
        ] {
            assert!(HTML.contains(needle), "the page has no {needle}");
        }
        let show = js_fn("showChat");
        assert!(show.contains("loadHistory("), "{show}");
        assert!(show.contains("watchNode("), "{show}");
        assert!(js_fn("sendChat").contains("type: 'send'"));
        assert!(js_fn("stopChat").contains("type: 'stop'"));
    }

    #[test]
    fn dispatcher_page_chat_uses_textcontent() {
        // Streamed and history text is data: it is drawn as text, never as markup.
        assert!(!HTML.contains("innerHTML"));
        assert!(!HTML.contains("insertAdjacentHTML"));
        assert!(js_fn("onChatEvent").contains("textContent"));
        assert!(js_fn("bubble").contains("el('div'"), "{}", js_fn("bubble"));
    }

    #[test]
    fn dispatcher_page_unwatches_on_back() {
        // Leaving the chat (Back, or the sidebar) drops the watch and the drafts.
        assert!(js_fn("hideChat").contains("dropChat()"));
        assert!(js_fn("back").contains("hideChat()"));
        let drop = js_fn("dropChat");
        assert!(drop.contains("c.off()"), "{drop}");
        assert!(drop.contains("drafts.clear()"), "{drop}");
    }

    #[test]
    fn dispatcher_page_reopen_drops_the_old_chat_first() {
        // A second open must not stack a second watch or draw the stream twice.
        let show = js_fn("showChat");
        let drop_at = show
            .find("dropChat()")
            .expect("showChat drops the old chat");
        let watch_at = show.find("watchNode(").expect("showChat watches");
        assert!(drop_at < watch_at, "{show}");
    }

    #[test]
    fn dispatcher_page_ask_node_is_generic_and_ask_is_unchanged() {
        assert!(js_fn("askNode").contains("fantastic.send(id, payload)"));
        assert!(js_fn("ask").contains("fantastic.send('dispatcher', payload)"));
    }

    fn load_full() -> Value {
        json!({
            "workers": {"busy": 1, "idle": 2, "stalled": 2},
            "tasks": {"held": 1, "in_review": 3, "done": 2, "blocked": 0, "queued": 3},
            "done_last_hour": 2,
            "digest_bytes_per_hour": 480,
            "driver_tokens_per_hour": 1200,
        })
    }

    fn sidebar_of(rows: &[Row], load: &Value) -> Value {
        let mut groups = BTreeMap::new();
        groups.insert("A".to_string(), vec!["w1".to_string(), "w2".to_string()]);
        sidebar(
            "team",
            &groups,
            rows,
            &subs_of(rows),
            &BTreeSet::new(),
            load,
        )
    }

    #[test]
    fn sidebar_load_carries_digest_bytes_and_driver_tokens_per_hour() {
        let sb = sidebar_of(&rows_w(), &load_full());
        assert_eq!(sb["load"]["digest_bytes_per_hour"], 480);
        assert_eq!(sb["load"]["driver_tokens_per_hour"], 1200);
        let mut bare = load_full();
        bare.as_object_mut()
            .unwrap()
            .remove("digest_bytes_per_hour");
        bare.as_object_mut()
            .unwrap()
            .remove("driver_tokens_per_hour");
        let sb = sidebar_of(&rows_w(), &bare);
        assert!(sb["load"].get("digest_bytes_per_hour").is_none(), "{sb}");
        assert!(sb["load"].get("driver_tokens_per_hour").is_none(), "{sb}");
    }

    #[test]
    fn html_footer_draws_cost_per_hour() {
        assert!(HTML.contains("digest_bytes_per_hour"));
        assert!(HTML.contains("driver_tokens_per_hour"));
        assert!(HTML.contains("tok/h"));
        assert!(HTML.contains("B/h"));
    }

    #[test]
    fn sidebar_worker_item_has_minutes_and_pr() {
        let mut rows = rows_w();
        rows[0].minutes = Some(2);
        rows[0].pr = Some(9);
        let sb = sidebar_of(&rows, &Value::Null);
        let items = group_items(&sb, "A");
        let w1 = items
            .iter()
            .find(|i| i["kind"] == "worker" && i["id"] == "w1")
            .unwrap();
        assert_eq!(w1["minutes"], 2);
        assert_eq!(w1["pr"], 9);
        let w2 = items
            .iter()
            .find(|i| i["kind"] == "worker" && i["id"] == "w2")
            .unwrap();
        assert!(w2["minutes"].is_null() && w2["pr"].is_null(), "{w2}");
    }

    #[test]
    fn html_worker_row_draws_minutes_and_pr() {
        let w = js_fn("workerRow");
        assert!(w.contains("it.minutes"), "{w}");
        assert!(w.contains("'m'"), "{w}");
        assert!(w.contains("it.pr"), "{w}");
        assert!(w.contains("'minutes'") && w.contains("'pr'"), "{w}");
        assert!(!w.contains("innerHTML"));
    }

    #[test]
    fn sidebar_overview_counts_prs_waiting_and_stalled() {
        let sb = sidebar_of(&rows_w(), &load_full());
        assert_eq!(sb["waiting"], json!({"prs": 3, "stalled": 2}));
    }

    #[test]
    fn sidebar_without_load_has_no_waiting() {
        let sb = sidebar_of(&rows_w(), &Value::Null);
        assert!(sb.get("waiting").is_none(), "{sb}");
    }

    #[test]
    fn sidebar_marks_a_stalled_worker() {
        let mut rows = rows_w();
        rows[0].state = "stalled 45 min".into();
        let sb = sidebar_of(&rows, &Value::Null);
        let items = group_items(&sb, "A");
        let get = |id: &str| {
            items
                .iter()
                .find(|i| i["kind"] == "worker" && i["id"] == id)
                .unwrap()
                .clone()
        };
        assert_eq!(get("w1")["stalled"], json!(true));
        assert_eq!(get("w2")["stalled"], json!(false));
    }

    #[test]
    fn html_draws_waiting_for_you() {
        assert!(HTML.contains("reply.waiting"));
        assert!(HTML.contains("el('div', 'team-waiting'"));
        assert!(HTML.contains("waiting for you: "));
        assert!(HTML.contains("it.stalled"));
        assert!(js_fn("renderSidebar").contains("reply.waiting"));
        // The waiting line comes before the footer.
        let r = js_fn("renderSidebar");
        assert!(r.find("team-waiting").unwrap() < r.find("'team-load'").unwrap());
    }

    #[test]
    fn control_accepts_hand_task_add_and_remove_worker_and_refuses_others() {
        for action in ["hand_task", "add_worker"] {
            assert_eq!(
                control_reply("w1", action, false)["sent"],
                json!(true),
                "{action}"
            );
        }
        let r = control_reply("w1", "remove_worker", true);
        assert_eq!(r["sent"], json!(true), "{r}");
        assert_eq!(r["action"], "remove_worker");
        for action in ["delete_agent", "reflect"] {
            let r = control_reply("w1", action, true);
            assert_eq!(r["sent"], json!(false), "{r}");
            assert!(r["error"].as_str().is_some(), "{r}");
        }
    }

    #[test]
    fn remove_worker_without_confirm_asks_first() {
        let r = control_reply("w1", "remove_worker", false);
        assert_eq!(r["sent"], json!(false));
        assert!(r["question"].as_str().unwrap().contains("w1"), "{r}");
    }

    #[test]
    fn groups_with_adds_a_member_and_refuses_unknown_group_or_duplicate() {
        let g = groups_a();
        let out = groups_with(&g, "A", "w9").unwrap();
        assert_eq!(out["A"], vec!["w1".to_string(), "w9".to_string()]);
        assert_eq!(g["A"], vec!["w1".to_string()], "the input is not changed");
        assert!(groups_with(&g, "nope", "w9")
            .unwrap_err()
            .contains("unknown group"));
        assert!(groups_with(&g, "A", "w1").unwrap_err().contains("already"));
    }

    #[test]
    fn groups_without_removes_a_member_everywhere() {
        let mut g = groups_a();
        g.insert("B".to_string(), vec!["w1".to_string(), "w2".to_string()]);
        let out = groups_without(&g, "w1");
        assert!(out["A"].is_empty());
        assert_eq!(out["B"], vec!["w2".to_string()]);
    }

    #[test]
    fn team_groups_with_keeps_the_driver_and_replaces_members() {
        let cur = json!({"A": {"driver": "lead", "members": ["w1"]}});
        let mut g = BTreeMap::new();
        g.insert("A".to_string(), vec!["w1".to_string(), "w9".to_string()]);
        let out = team_groups_with(&cur, &g);
        assert_eq!(out["A"], json!({"driver": "lead", "members": ["w1", "w9"]}));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn hand_task_needs_a_task_text() {
        use fantastic_kernel::bootstrap;
        let dir = tempfile::TempDir::new().unwrap();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let kernel = kernel_with_probe(dir.path(), seen.clone()).await;
        for payload in [
            json!({"type": "control", "node": "probe", "action": "hand_task"}),
            json!({"type": "control", "node": "probe", "action": "hand_task", "task": "  "}),
        ] {
            let out = ask_kernel(&kernel, "dispatcher", payload).await;
            assert_eq!(out["sent"], json!(false), "{out}");
            assert!(
                out["error"].as_str().unwrap().contains("needs a task"),
                "{out}"
            );
        }
        assert!(seen.lock().unwrap().is_empty());
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn control_via_kernel_hand_task_reaches_the_worker() {
        use fantastic_kernel::bootstrap;
        let dir = tempfile::TempDir::new().unwrap();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let kernel = kernel_with_probe(dir.path(), seen.clone()).await;
        let out = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "control", "node": "probe", "action": "hand_task", "task": "fix r#5"}),
        )
        .await;
        assert_eq!(out["sent"], json!(true), "{out}");
        assert_eq!(
            *seen.lock().unwrap(),
            vec![json!({"type": "send", "text": "task fix r#5"})]
        );
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    /// A kernel with a dispatcher as the plan makes it (the agent verbs on the
    /// root), a board record holding `team_groups`, and one group `g` with `w1`.
    async fn kernel_with_groups(dir: &std::path::Path) -> Arc<fantastic_kernel::Kernel> {
        use fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
        use fantastic_kernel::Item;
        let kernel = crate::start(dir).await.unwrap();
        let can_call = json!({ (DEFAULT_ROOT_ID): crate::tree::DISPATCHER_KERNEL_VERBS });
        for item in [
            json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "dispatcher",
                "handler_module": HANDLER_MODULE, "can_call": can_call,
                "groups": {"g": ["w1"]}}),
            json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "board",
                "handler_module": "ram_store.tools",
                "team_groups": {"g": {"driver": "lead", "members": ["w1"]}}}),
            // the team's one store agent, as the plan makes it
            crate::config::store_agent_record(
                &crate::config::StoreCfg::default(),
                &["lead".to_string(), "w1".to_string()],
            ),
        ] {
            let o = kernel.send(vec![Item::new(DEFAULT_ROOT_ID, item)]).await;
            assert!(o[0].result.get("error").is_none(), "{:?}", o[0].result);
        }
        crate::tree::grant_agent_verbs(&kernel).unwrap();
        kernel
    }

    fn meta_of(kernel: &Arc<fantastic_kernel::Kernel>, id: &str) -> serde_json::Map<String, Value> {
        kernel
            .agents
            .get(&fantastic_kernel::AgentId::from(id))
            .map(|a| a.meta.read().unwrap().clone())
            .unwrap_or_default()
    }

    const MINI_TEAM: &str = "templates:\n  t:\n    handler_module: ram_store.tools\n    model: m\n    endpoint: http://e\nfirst_agent:\n  template: t\nqueue:\n  id: q\n";

    /// Serialises the tests that change the process's working directory.
    static CWD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[tokio::test(flavor = "multi_thread")]
    async fn control_via_kernel_add_worker_creates_the_agent_and_lists_it_in_the_tree() {
        use fantastic_kernel::bootstrap;
        let _cwd = CWD.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("team.yaml"), MINI_TEAM).unwrap();
        let kernel = kernel_with_groups(dir.path()).await;
        let before = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let out = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "control", "node": "g", "action": "add_worker", "id": "w2", "template": "t"}),
        )
        .await;
        // An unknown template and a duplicate id are refused, nothing is created.
        let bad = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "control", "node": "g", "action": "add_worker", "id": "w3", "template": "nope"}),
        )
        .await;
        let dup = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "control", "node": "g", "action": "add_worker", "id": "w1", "template": "t"}),
        )
        .await;
        std::env::set_current_dir(before).unwrap();
        assert_eq!(out["sent"], json!(true), "{out}");
        assert_eq!(bad["sent"], json!(false), "{bad}");
        assert!(
            bad["error"].as_str().unwrap().contains("no template"),
            "{bad}"
        );
        assert_eq!(dup["sent"], json!(false), "{dup}");
        assert!(kernel
            .agents
            .contains_key(&fantastic_kernel::AgentId::from("w2")));
        assert!(!kernel
            .agents
            .contains_key(&fantastic_kernel::AgentId::from("w2_memory")));
        assert!(!kernel
            .agents
            .contains_key(&fantastic_kernel::AgentId::from("w3")));
        assert_eq!(
            meta_of(&kernel, "dispatcher")["groups"],
            json!({"g": ["w1", "w2"]})
        );
        assert_eq!(
            meta_of(&kernel, "board")["team_groups"],
            json!({"g": {"driver": "lead", "members": ["w1", "w2"]}})
        );
        let tree = ask_kernel(&kernel, "dispatcher", json!({"type": "tree"})).await;
        let ids: Vec<&str> = tree["children"][0]["children"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["row"]["id"].as_str().unwrap())
            .collect();
        assert!(ids.contains(&"w2"), "{tree}");
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn control_via_kernel_add_worker_grants_the_dispatcher_the_new_worker() {
        use fantastic_kernel::bootstrap;
        let _cwd = CWD.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("team.yaml"), MINI_TEAM).unwrap();
        let kernel = kernel_with_groups(dir.path()).await;
        let before = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let added = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "control", "node": "g", "action": "add_worker", "id": "w2", "template": "t"}),
        )
        .await;
        std::env::set_current_dir(before).unwrap();
        assert_eq!(added["sent"], json!(true), "{added}");
        let verbs = json!(crate::tree::CONTROL_VERBS);
        assert_eq!(meta_of(&kernel, "w2")["callable_by"]["dispatcher"], verbs);
        assert_eq!(meta_of(&kernel, "dispatcher")["can_call"]["w2"], verbs);
        // The stop now reaches the worker's own handler (which may not know the verb)
        // instead of being refused at the edge.
        let stop = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "control", "node": "w2", "action": "stop", "confirm": true}),
        )
        .await;
        let err = stop["error"].as_str().unwrap_or("");
        assert!(!err.contains("may not call"), "{stop}");
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_worker_uses_the_shared_store_and_makes_no_memory() {
        use fantastic_kernel::bootstrap;
        let _cwd = CWD.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("team.yaml"), MINI_TEAM).unwrap();
        let kernel = kernel_with_groups(dir.path()).await;
        let before = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let out = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "control", "node": "g", "action": "add_worker", "id": "w2", "template": "t"}),
        )
        .await;
        std::env::set_current_dir(before).unwrap();
        assert_eq!(out["sent"], json!(true), "{out}");
        let ids: Vec<String> = kernel.agents.iter().map(|e| e.key().0.clone()).collect();
        assert!(!ids.iter().any(|i| i == "w2_memory"), "{ids:?}");
        // the kernel's own bookkeeping store is the only `_memory` there is
        assert!(ids
            .iter()
            .filter(|i| i.ends_with("_memory"))
            .all(|i| i == "kernel_memory"));
        let w = meta_of(&kernel, "w2");
        assert_eq!(w["memory_store_id"], json!("memory"));
        assert_eq!(w["can_call"]["memory"], json!("*"));
        let store = meta_of(&kernel, "memory");
        assert_eq!(store["callable_by"]["w2"], json!("*"), "{store:?}");
        assert_eq!(store["callable_by"]["lead"], json!("*"), "{store:?}");
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_worker_without_the_store_is_refused_and_leaves_no_worker() {
        use fantastic_kernel::bootstrap;
        let _cwd = CWD.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("team.yaml"), MINI_TEAM).unwrap();
        let kernel = kernel_with_groups(dir.path()).await;
        let gone = ask_kernel(
            &kernel,
            "kernel_state",
            json!({"type": "delete_agent", "id": "memory"}),
        )
        .await;
        assert!(gone.get("error").is_none(), "{gone}");
        let before = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let out = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "control", "node": "g", "action": "add_worker", "id": "w2", "template": "t"}),
        )
        .await;
        std::env::set_current_dir(before).unwrap();
        assert_eq!(out["sent"], json!(false), "{out}");
        assert!(
            out["error"]
                .as_str()
                .unwrap()
                .contains("the store agent `memory` does not exist"),
            "{out}"
        );
        assert!(!kernel
            .agents
            .contains_key(&fantastic_kernel::AgentId::from("w2")));
        assert_eq!(
            meta_of(&kernel, "dispatcher")["groups"],
            json!({"g": ["w1"]})
        );
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn remove_worker_keeps_the_store() {
        use fantastic_kernel::bootstrap;
        use fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
        use fantastic_kernel::Item;
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_with_groups(dir.path()).await;
        kernel
            .send(vec![Item::new(
                DEFAULT_ROOT_ID,
                json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "w1", "handler_module": "ram_store.tools"}),
            )])
            .await;
        let out = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "control", "node": "w1", "action": "remove_worker", "confirm": true}),
        )
        .await;
        assert_eq!(out["sent"], json!(true), "{out}");
        assert!(!kernel
            .agents
            .contains_key(&fantastic_kernel::AgentId::from("w1")));
        assert!(kernel
            .agents
            .contains_key(&fantastic_kernel::AgentId::from("memory")));
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn control_via_kernel_remove_worker_deletes_it_and_drops_it_from_the_groups() {
        use fantastic_kernel::bootstrap;
        use fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
        use fantastic_kernel::Item;
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_with_groups(dir.path()).await;
        for item in [
            json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "w1", "handler_module": "ram_store.tools"}),
        ] {
            kernel.send(vec![Item::new(DEFAULT_ROOT_ID, item)]).await;
        }
        let asked = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "control", "node": "w1", "action": "remove_worker"}),
        )
        .await;
        assert!(asked["question"].as_str().is_some(), "{asked}");
        assert!(kernel
            .agents
            .contains_key(&fantastic_kernel::AgentId::from("w1")));
        let out = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "control", "node": "w1", "action": "remove_worker", "confirm": true}),
        )
        .await;
        assert_eq!(out["sent"], json!(true), "{out}");
        assert!(!kernel
            .agents
            .contains_key(&fantastic_kernel::AgentId::from("w1")));
        assert_eq!(meta_of(&kernel, "dispatcher")["groups"], json!({"g": []}));
        assert_eq!(
            meta_of(&kernel, "board")["team_groups"],
            json!({"g": {"driver": "lead", "members": []}})
        );
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[test]
    fn html_has_hand_task_add_and_remove_controls() {
        let w = js_fn("workerRow");
        assert!(
            w.contains("hand_task") && w.contains("remove_worker"),
            "{w}"
        );
        assert!(HTML.contains("window.prompt("));
        assert!(js_fn("handTask").contains("'hand_task'"));
        let form = js_fn("addWorkerForm");
        assert!(form.contains("'add_worker'"), "{form}");
        assert!(form.contains("template"), "{form}");
        assert!(js_fn("renderSidebar").contains("addWorkerForm(g.id)"));
        // The confirm for a remove goes through the same question as stop.
        assert!(js_fn("control").contains("reply.question"));
        assert!(!HTML.contains("innerHTML"));
    }

    /// A sidebar answers with the cadencer's load (cost per hour) when it replies.
    struct LoadBundle;

    #[async_trait::async_trait]
    impl fantastic_kernel::bundle::Bundle for LoadBundle {
        fn name(&self) -> &str {
            "load_test.tools"
        }

        async fn handle(
            &self,
            _agent_id: &fantastic_kernel::AgentId,
            payload: &Value,
            _kernel: &Arc<fantastic_kernel::Kernel>,
        ) -> Result<fantastic_kernel::bundle::Reply, fantastic_kernel::bundle::BundleError>
        {
            Ok(match payload["type"].as_str() {
                Some("status") => Some(json!({"load": {
                    "tasks": {"in_review": 2}, "workers": {"stalled": 1},
                    "digest_bytes_per_hour": 99, "driver_tokens_per_hour": 7}})),
                Some("rows") => Some(json!({"rows": [], "load": {"tasks": {"in_review": 2}}})),
                _ => None,
            })
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sidebar_via_kernel_takes_the_cost_per_hour_from_the_cadencer() {
        use fantastic_kernel::bootstrap::{self, BootstrapOptions, DEFAULT_ROOT_ID};
        use fantastic_kernel::Item;
        let dir = tempfile::TempDir::new().unwrap();
        let mut reg = crate::registry();
        reg.register("load_test.tools", LoadBundle);
        let mut booted = bootstrap::bootstrap(reg, BootstrapOptions::one_shot(dir.path())).unwrap();
        fantastic_kernel::lock::acquire(dir.path()).unwrap();
        booted.write_seed().unwrap();
        let kernel = booted.kernel.clone();
        crate::boot_kernel(&kernel).await.unwrap();
        for item in [
            json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "dispatcher",
                "handler_module": HANDLER_MODULE, "can_call": {"board": "*", "cadencer": "*"}}),
            json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "board",
                "handler_module": "load_test.tools", "callable_by": {"dispatcher": "*"}}),
            json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "cadencer",
                "handler_module": "load_test.tools", "callable_by": {"dispatcher": "*"}}),
        ] {
            kernel.send(vec![Item::new(DEFAULT_ROOT_ID, item)]).await;
        }
        let sb = ask_kernel(&kernel, "dispatcher", json!({"type": "sidebar"})).await;
        assert_eq!(sb["load"]["digest_bytes_per_hour"], 99, "{sb}");
        assert_eq!(sb["load"]["driver_tokens_per_hour"], 7, "{sb}");
        assert_eq!(sb["waiting"], json!({"prs": 2, "stalled": 1}), "{sb}");
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    /// A registry stand-in: answers `get_count` with {running: 2, max: 3}, or with a
    /// refusal when its `fail` field is set.
    struct CountBundle;

    #[async_trait::async_trait]
    impl fantastic_kernel::bundle::Bundle for CountBundle {
        fn name(&self) -> &str {
            "count_test.tools"
        }

        async fn handle(
            &self,
            agent_id: &fantastic_kernel::AgentId,
            payload: &Value,
            kernel: &Arc<fantastic_kernel::Kernel>,
        ) -> Result<fantastic_kernel::bundle::Reply, fantastic_kernel::bundle::BundleError>
        {
            let fail = kernel
                .agents
                .get(agent_id)
                .is_some_and(|a| a.meta.read().unwrap().get("fail").is_some());
            Ok(match payload["type"].as_str() {
                Some("get_count") if fail => Some(json!({"error": "disabled"})),
                Some("get_count") => Some(json!({"running": 2, "max": 3})),
                _ => None,
            })
        }
    }

    /// A kernel with a dispatcher (its `roles` meta set to `roles`, when given) and a
    /// registry stand-in `reg` (it refuses `get_count` when `fail`).
    async fn kernel_with_registry(
        dir: &std::path::Path,
        roles: Option<&str>,
        fail: bool,
    ) -> Arc<fantastic_kernel::Kernel> {
        use fantastic_kernel::bootstrap::{self, BootstrapOptions, DEFAULT_ROOT_ID};
        use fantastic_kernel::Item;
        let mut reg = crate::registry();
        reg.register("count_test.tools", CountBundle);
        let mut booted = bootstrap::bootstrap(reg, BootstrapOptions::one_shot(dir)).unwrap();
        fantastic_kernel::lock::acquire(dir).unwrap();
        booted.write_seed().unwrap();
        let kernel = booted.kernel.clone();
        crate::boot_kernel(&kernel).await.unwrap();
        let mut d = json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "dispatcher",
            "handler_module": HANDLER_MODULE, "can_call": {"reg": "*"}});
        if let Some(r) = roles {
            d["roles"] = json!(r);
        }
        let mut r = json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "reg",
            "handler_module": "count_test.tools", "callable_by": {"dispatcher": "*"}});
        if fail {
            r["fail"] = json!(true);
        }
        for item in [d, r] {
            kernel.send(vec![Item::new(DEFAULT_ROOT_ID, item)]).await;
        }
        kernel
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sidebar_carries_fleet_when_a_registry_answers() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_with_registry(dir.path(), Some("reg"), false).await;
        let sb = ask_kernel(&kernel, "dispatcher", json!({"type": "sidebar"})).await;
        assert_eq!(sb["fleet"], json!({"running": 2, "max": 3}), "{sb}");
        kernel.shutdown().await;
        fantastic_kernel::bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sidebar_has_no_fleet_without_a_registry() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_with_registry(dir.path(), None, false).await;
        let sb = ask_kernel(&kernel, "dispatcher", json!({"type": "sidebar"})).await;
        assert!(sb.get("fleet").is_none(), "{sb}");
        assert!(sb["team"].is_string(), "{sb}");
        kernel.shutdown().await;
        fantastic_kernel::bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sidebar_has_no_fleet_when_the_registry_refuses() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_with_registry(dir.path(), Some("reg"), true).await;
        let sb = ask_kernel(&kernel, "dispatcher", json!({"type": "sidebar"})).await;
        assert!(sb.get("fleet").is_none(), "{sb}");
        kernel.shutdown().await;
        fantastic_kernel::bootstrap::shutdown(dir.path()).unwrap();
    }

    #[test]
    fn fleet_of_reads_a_count_and_ignores_errors() {
        assert_eq!(
            fleet_of(&json!({"running": 1, "max": null})),
            Some(json!({"running": 1, "max": null}))
        );
        assert_eq!(
            fleet_of(&json!({"running": 0})),
            Some(json!({"running": 0, "max": null}))
        );
        assert_eq!(fleet_of(&json!({"error": "x", "running": 1})), None);
        assert_eq!(fleet_of(&json!({})), None);
    }

    #[test]
    fn html_footer_shows_agents_count() {
        assert!(HTML.contains("' · agents '"));
        assert!(HTML.contains("reply.fleet.running"));
        assert!(HTML.contains("reply.fleet.max"));
        assert!(!HTML.contains("innerHTML"));
    }

    // ---- the page's chat surface, run in node against a stub DOM ----

    /// Run one scenario of `tests/js/chat_harness.js` on the page. None (and a
    /// notice) when node is missing; with `CI` set a missing node is a failure.
    fn run_node(scenario: &str) -> Option<Value> {
        let root = env!("CARGO_MANIFEST_DIR");
        let out = match std::process::Command::new("node")
            .arg(format!("{root}/tests/js/chat_harness.js"))
            .arg(format!("{root}/web/dispatcher.html"))
            .arg(scenario)
            .output()
        {
            Ok(o) => o,
            Err(e) => {
                assert!(
                    std::env::var_os("CI").is_none(),
                    "node is required in CI: {e}"
                );
                eprintln!("skipped `{scenario}`: node is not installed ({e})");
                return None;
            }
        };
        assert!(
            out.status.success(),
            "scenario {scenario} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        Some(serde_json::from_slice(&out.stdout).expect("the harness prints JSON"))
    }

    #[test]
    fn chat_stream_draws_text_once() {
        let Some(v) = run_node("text_once") else {
            return;
        };
        assert_eq!(v["msgs"], json!([{"cls": "msg assistant", "text": "ab"}]));
    }

    #[test]
    fn chat_draws_tool_block_from_status() {
        let Some(v) = run_node("tool_block") else {
            return;
        };
        assert_eq!(v["first"].as_array().unwrap().len(), 1);
        assert!(v["first"][0]["text"].as_str().unwrap().contains("git(g)"));
        assert_eq!(v["first"][0]["state"], "pending");
        assert_eq!(
            v["second"].as_array().unwrap().len(),
            1,
            "one block per call_id"
        );
        assert_eq!(v["second"][0]["state"], "done");
        assert!(v["second"][0]["text"]
            .as_str()
            .unwrap()
            .contains("all fine"));
        assert_eq!(v["third"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn chat_draws_inquiry_buttons_and_sends_answer() {
        let Some(v) = run_node("inquiries") else {
            return;
        };
        assert_eq!(v["onOpen"], 1, "read once on open");
        assert_eq!(v["beforeTimer"], 1, "a done is debounced");
        assert_eq!(v["asked"][1], json!({"type": "inquiries", "asker": "w1"}));
        assert_eq!(v["labels5"], json!(["Yes", "No"]));
        assert_eq!(v["labels6"], json!(["alpha", "beta", "gamma"]));
        assert_eq!(v["disabledWhileSending"], json!([true, true]));
        assert_eq!(v["afterError"]["disabled"], json!([false, false]));
        assert!(v["afterError"]["error"]
            .as_str()
            .unwrap()
            .contains("no open inquiry"));
        assert_eq!(v["afterOk"], "Proceed?you answered: Yes");
        assert_eq!(
            v["answers"][1],
            json!({"id": "user", "payload": {"type": "answer_inquiry", "re": 5, "answer": true}})
        );
        assert_eq!(v["cards"], 2, "the same re is not drawn twice");
        assert_eq!(
            v["singleAnswer"],
            json!({"type": "answer_inquiry", "re": 6, "answer": "beta"})
        );
        let k = run_node("inquiry_kinds").unwrap();
        assert_eq!(k["answers"][0]["answer"], json!(["a", "c"]));
        assert_eq!(k["answers"][1]["answer"], "Ada");
    }

    #[test]
    fn chat_draws_context_stats() {
        let Some(v) = run_node("stats") else { return };
        assert_eq!(v["plain"], "25 tok/s \u{b7} 12k / 32k ctx");
        assert!(v["estimated"].as_str().unwrap().contains("~12k"));
    }

    #[test]
    fn chat_sends_attachment_as_text() {
        let Some(v) = run_node("attach") else { return };
        assert_eq!(v["hasFileInput"], true);
        assert_eq!(v["chip"], "a.txt\u{d7}");
        assert_eq!(v["keys"], json!(["client_id", "text", "type"]));
        let text = v["text"].as_str().unwrap();
        assert!(
            text.contains("see") && text.contains("[attached: a.txt]") && text.contains("hello")
        );
        assert_eq!(
            v["bubble"], "seea.txt",
            "the bubble holds the typed text and the name only"
        );
        assert_eq!(v["chipAfter"], "");
        assert_eq!(v["alone"], "[attached: b.txt]\nyo");
        assert_eq!(v["sendsAfterBig"], 0);
        assert!(v["bigError"]
            .as_str()
            .unwrap()
            .contains("larger than 200000"));
        assert_eq!(v["bigChip"], "");
        assert!(v["bigAttached"].is_null());
        assert!(v["readError"]
            .as_str()
            .unwrap()
            .contains("could not be read"));
        assert!(v["badAttached"].is_null());
    }

    #[test]
    fn chat_draws_reasoning_block() {
        let Some(v) = run_node("reasoning") else {
            return;
        };
        let blocks = v["blocks"].as_array().unwrap();
        assert_eq!(blocks.len(), 2, "the empty thought is removed");
        assert_eq!(blocks[0]["body"], "let me think");
        assert!(blocks[0]["summary"]
            .as_str()
            .unwrap()
            .starts_with("thought for"));
        assert!(blocks[1]["cls"].as_str().unwrap().contains("not-sent"));
        assert_eq!(v["assistant"], 0, "no text bubble for a thought or a call");
        assert_eq!(v["total"], 2);
    }

    #[test]
    fn chat_status_replays_a_running_turn() {
        let Some(v) = run_node("status_replay") else {
            return;
        };
        let m = &v["mine"];
        assert_eq!(m["statusCall"][0]["type"], "status");
        assert!(m["statusCall"][0]["client_id"]
            .as_str()
            .unwrap()
            .starts_with("team_"));
        let msgs = m["msgs"].as_array().unwrap();
        assert_eq!(msgs[0], json!({"cls": "msg user", "text": "q"}));
        assert_eq!(msgs[1], json!({"cls": "msg assistant", "text": "par"}));
        assert_eq!(m["tools"], 1);
        assert!(msgs[2]["text"].as_str().unwrap().contains("git(g)"));
        assert!(msgs[2]["text"].as_str().unwrap().contains("ok"));
        assert_eq!(msgs[3]["text"], "next one");
        assert_eq!(m["disabled"], true);
        assert_eq!(m["disabledAfterDone"], false);
        for k in ["notMine", "none"] {
            assert_eq!(v[k]["msgs"], json!([]), "{k}");
            assert_eq!(v[k]["disabled"], false, "{k}");
        }
        let e = run_node("status_error").unwrap();
        assert_eq!(e["msgs"], json!([]));
        assert_eq!(e["disabled"], false);
    }

    #[test]
    fn page_mentions_new_chat_surface() {
        for want in [
            "type: 'status'",
            "type: 'inquiries'",
            "type: 'answer_inquiry'",
            "kind === 'thinking'",
        ] {
            assert!(HTML.contains(want), "page lacks {want}");
        }
        assert!(
            HTML.contains("type=\"file\"") || HTML.contains("file.type = 'file'"),
            "page lacks a file input"
        );
        assert!(!HTML.contains("innerHTML"));
    }

    #[test]
    fn row_text_shows_minutes() {
        let Some(v) = run_node("rows") else { return };
        assert!(v["full"]["text"].as_str().unwrap().contains("2m"));
        assert_eq!(v["full"]["minutes"], "2m");
        assert!(v["empty"]["minutes"].is_null());
        assert!(!v["empty"]["text"].as_str().unwrap().contains("0m"));
        assert_eq!(v["zero"]["minutes"], "0m");
    }

    #[test]
    fn row_text_shows_pr_number() {
        let Some(v) = run_node("rows") else { return };
        assert!(v["full"]["text"].as_str().unwrap().contains("PR #12"));
        assert!(v["empty"]["pr"].is_null());
        assert!(!v["empty"]["text"].as_str().unwrap().contains("PR #"));
        assert!(v["zero"]["pr"].is_null());
    }
}
