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
/// What the bundle is for: its one short man text; how to call it is rendered from its
/// declared verbs (`docs`, `reflect`).
const MAN: &str = "The main view of the team's web UI: the sidebar (workers per group, their subagents, the team's services), the tree of the team, what to open for a node, and the control of a worker or group. It only reads and asks: no verb assigns work.";

pub mod record;
pub mod verbs;

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
/// `load` is the roles registry's `load` object (`Null` when none came). The team gets `load`
/// {tasks, done_last_hour} when one came, plus the cadencer's
/// `digest_bytes_per_hour` when `load` carries it,
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
        let group = json!({"id": gid, "items": items});
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
        if load
            .get("digest_bytes_per_hour")
            .is_some_and(|v| !v.is_null())
        {
            team_load["digest_bytes_per_hour"] = load["digest_bytes_per_hour"].clone();
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
/// itself, over the chat verbs granted by `tree::grant_chat_to`: it loads the history,
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

/// `stop` and `remove_worker` need an explicit confirm; without it the answer is a question.
pub fn control_reply(node: &str, action: verbs::ControlAction, confirm: bool) -> Value {
    use verbs::ControlAction::{RemoveWorker, Stop};
    if action == Stop && !confirm {
        return json!({
            "question": format!("Stop {node}? Send again with confirm: true."),
            "sent": false,
        });
    }
    if action == RemoveWorker && !confirm {
        return json!({
            "question": format!("Remove {node}? Send again with confirm: true."),
            "sent": false,
        });
    }
    json!({"sent": true, "node": node, "action": action.as_str()})
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
    rows_from(&snap, groups, limit, now)
}

/// [`rows_of`] from a board snapshot already in hand.
fn rows_from(
    snap: &Value,
    groups: &BTreeMap<String, Vec<String>>,
    limit: u64,
    now: u64,
) -> Vec<Row> {
    let mut ids: Vec<String> = groups.values().flatten().cloned().collect();
    if let Some(agents) = snap["agents"].as_object() {
        for k in agents.keys() {
            if !ids.contains(k) {
                ids.push(k.clone());
            }
        }
    }
    ids.iter().map(|w| row_of(w, snap, now, limit)).collect()
}

/// How many levels the page indents an agent row at most.
pub const INDENT_CAP: usize = 4;

/// The agents of roles mode as one flat depth-first list for the page, from the board's
/// `roles` section: the lead first (`lead: true`, label "Lead"), then every node of the
/// tree in its child order. Each item is {id, role, state, depth, indent, guides, lead,
/// label, has_children}; `indent` and `guides` stop at [`INDENT_CAP`], `depth` keeps counting. With no
/// tree but a lead id, just the lead; with neither, nothing.
pub fn agent_list(roles: &Value) -> Vec<Value> {
    fn item(node: &Value, depth: usize, lead: bool) -> Value {
        let id = node["id"].as_str().unwrap_or("");
        let role = node["role"].as_str().unwrap_or("");
        let state = node["state"].as_str().unwrap_or("active");
        let indent = depth.min(INDENT_CAP);
        let label = if lead {
            "Lead"
        } else if role.is_empty() {
            id
        } else {
            role
        };
        json!({"id": id, "role": role, "state": state, "depth": depth, "indent": indent,
            "guides": indent, "lead": lead, "label": label})
    }
    fn walk(node: &Value, depth: usize, out: &mut Vec<Value>) {
        out.push(item(node, depth, false));
        for k in node["children"].as_array().into_iter().flatten() {
            walk(k, depth + 1, out);
        }
    }
    let lead = roles["lead"].as_str().filter(|l| !l.is_empty());
    let tree = &roles["tree"];
    let mut out = Vec::new();
    if tree.get("id").and_then(Value::as_str).is_some() {
        let root_is_lead = lead.is_none_or(|l| tree["id"].as_str() == Some(l));
        if root_is_lead {
            let mut root = item(tree, 0, true);
            if root["role"] == "" {
                root["role"] = json!("project_lead");
            }
            out.push(root);
            for k in tree["children"].as_array().into_iter().flatten() {
                walk(k, 1, &mut out);
            }
        } else {
            walk(tree, 0, &mut out);
        }
    } else if let Some(l) = lead {
        out.push(item(
            &json!({"id": l, "role": "project_lead", "state": "active"}),
            0,
            true,
        ));
    }
    // The lead is always first, wherever the tree put it.
    if let Some(l) = lead {
        if let Some(i) = out.iter().position(|a| a["id"] == l) {
            let mut a = out.remove(i);
            a["lead"] = json!(true);
            a["label"] = json!("Lead");
            out.insert(0, a);
        }
    }
    // Derived from the list itself: a row has children when the next row is deeper.
    // The page uses it to collapse a subtree; it is not a record field.
    let depths: Vec<u64> = out
        .iter()
        .map(|a| a["depth"].as_u64().unwrap_or(0))
        .collect();
    for (i, a) in out.iter_mut().enumerate() {
        let deeper = depths.get(i + 1).is_some_and(|d| *d > depths[i]);
        a["has_children"] = json!(deeper);
    }
    out
}

/// The plan the lead's pane draws, from the board's `roles` section: {lead, fleet,
/// tasks}; each task is {id, source, holder, status} with the holder as text (an agent id,
/// or "subtree:<id>").
pub fn plan_of(roles: &Value) -> Value {
    let tasks: Vec<Value> = roles["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            let holder = match &t["holder"] {
                Value::String(h) => h.clone(),
                Value::Null => String::new(),
                other => match other.get("subtree").and_then(Value::as_str) {
                    Some(r) => format!("subtree:{r}"),
                    None => other.to_string(),
                },
            };
            json!({"id": t["id"], "source": t["source"], "holder": holder, "status": t["status"]})
        })
        .collect();
    json!({"lead": roles["lead"], "fleet": roles["fleet"], "tasks": tasks})
}

/// Let the page's legs chat with the lead and its agents: the chat verbs only, never `*`.
fn grant_agents(kernel: &Arc<fantastic_kernel::Kernel>, agents: &[Value]) {
    let ids: Vec<String> = agents
        .iter()
        .filter_map(|a| a["id"].as_str().map(str::to_string))
        .collect();
    if ids.is_empty() {
        return;
    }
    crate::tree::grant_chat_to(kernel, &ids, crate::page::WS_ID);
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

/// The workers the page has expanded, from the call's `expanded` list.
fn expanded_of(expanded: &Option<Vec<String>>) -> BTreeSet<String> {
    expanded.iter().flatten().cloned().collect()
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

/// Write the new membership to the dispatcher's own record, through the kernel's
/// `update_agent`.
/// Where the groups are kept: the dispatcher's own record (`me`), and the groups as they stand.
struct Membership<'a> {
    me: &'a str,
    roles: Option<&'a str>,
    groups: &'a BTreeMap<String, Vec<String>>,
}

async fn save_groups(
    peers: &dyn Peers,
    at: &Membership<'_>,
    groups: &BTreeMap<String, Vec<String>>,
) -> Result<(), String> {
    let me = at.me;
    let root = fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
    let r = peers
        .ask(
            root,
            json!({"type": "update_agent", "id": me, "groups": groups}),
        )
        .await;
    match fantastic_kernel::error_text(&r) {
        Some(e) => Err(format!("could not update the groups: {e}")),
        None => Ok(()),
    }
}

/// The tier `name` of the roles registry `roles` (the registry's own `tiers` field), as
/// the record fields of an AI agent. `Err` names the tiers the registry has.
fn tier_of(
    kernel: &Arc<fantastic_kernel::Kernel>,
    roles: Option<&str>,
    name: &str,
) -> Result<serde_json::Map<String, Value>, String> {
    let Some(roles) = roles else {
        return Err(
            "team_dispatcher: this dispatcher's record names no `roles` registry, so \
                    it has no tiers to make a worker from"
                .into(),
        );
    };
    let tiers = kernel
        .agents
        .get(&fantastic_kernel::AgentId::from(roles))
        .and_then(|e| e.meta.read().expect("meta").get("tiers").cloned())
        .and_then(|t| t.as_object().cloned())
        .ok_or_else(|| format!("team_dispatcher: the registry '{roles}' has no `tiers`"))?;
    match tiers.get(name).and_then(Value::as_object) {
        Some(t) => Ok(t.clone()),
        None => {
            let names: Vec<&str> = tiers.keys().map(String::as_str).collect();
            Err(format!(
                "team_dispatcher: no tier '{name}' in the registry '{roles}' (have: {})",
                names.join(", ")
            ))
        }
    }
}

/// `add_worker {node: group, id, template}`: create the worker from the tier `template`
/// of the roles registry's record (`tiers`, in the dispatcher's `roles` field), keep
/// its memory in the store that tier names (and let it call that store), and list it.
/// No memory agent is made.
async fn add_worker(
    kernel: &Arc<fantastic_kernel::Kernel>,
    peers: &dyn Peers,
    at: &Membership<'_>,
    group: &str,
    asked: (&Option<String>, &Option<String>),
    reply: Value,
) -> Value {
    let action = "add_worker";
    let id = asked.0.as_deref().unwrap_or("").trim().to_string();
    let template = asked.1.as_deref().unwrap_or("").trim().to_string();
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
    let tier = match tier_of(kernel, at.roles, &template) {
        Ok(t) => t,
        Err(e) => return refused(group, action, e),
    };
    let Some(store) = tier
        .get("memory_store_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
    else {
        return refused(
            group,
            action,
            format!("team_dispatcher: the tier '{template}' names no memory_store_id"),
        );
    };
    let root = fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
    let mut rec = serde_json::Map::new();
    rec.insert("type".into(), json!("create_agent"));
    rec.insert("parent_id".into(), json!(root));
    rec.insert("id".into(), json!(id));
    for (k, v) in &tier {
        rec.insert(k.clone(), v.clone());
    }
    rec.insert(
        "can_call".into(),
        json!({"$self": "*", "$subtree": "*", (store.as_str()): "*"}),
    );
    rec.insert("callable_by".into(), json!({"$self": "*", "$parent": "*"}));
    if let Some(e) = fantastic_kernel::error_text(&peers.ask(root, Value::Object(rec)).await) {
        return refused(group, action, format!("could not create '{id}': {e}"));
    }
    if let Err(e) = crate::tree::grant_store(kernel, &store, &id) {
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
    if let Some(e) = fantastic_kernel::error_text(&r) {
        return refused(worker, action, format!("could not remove '{worker}': {e}"));
    }
    let left = groups_without(at.groups, worker);
    if let Err(e) = save_groups(peers, at, &left).await {
        return refused(worker, action, e);
    }
    reply
}

/// The sidebar reply without `services`: the groups, workers, load, and in roles mode the
/// fleet, the agents and the plan.
#[allow(clippy::too_many_arguments)]
async fn sidebar_reply(
    kernel: &Arc<fantastic_kernel::Kernel>,
    peers: &KernelPeers,
    meta: &serde_json::Map<String, Value>,
    board: &str,
    team: &str,
    groups: &BTreeMap<String, Vec<String>>,
    limit: u64,
    now: u64,
    expanded: &Option<Vec<String>>,
) -> Value {
    let snap = peers.ask(board, json!({"type": "board"})).await;
    let rows = rows_from(&snap, groups, limit, now);
    let agents = agent_table(kernel);
    let subs: BTreeMap<String, Vec<Sub>> = rows
        .iter()
        .map(|r| (r.id.clone(), subagents_of(&agents, &r.id)))
        .collect();
    let mut load = Value::Null;
    // The cadencer's load holds the task counts plus the cost per hour.
    let cad = peers
        .ask(crate::tree::CADENCER_ID, json!({"type": "status"}))
        .await;
    if cad.get("error").is_none() && cad["load"].is_object() {
        load = cad["load"].clone();
    }
    let mut out = sidebar(team, groups, &rows, &subs, &expanded_of(expanded), &load);
    // The roles registry's count (agents running, and the cap), when the
    // team has a registry and it answers.
    if let Some(roles) = meta.get("roles").and_then(Value::as_str) {
        let c = peers.ask(roles, json!({"type": "get_count"})).await;
        if let Some(f) = fleet_of(&c) {
            out["fleet"] = f;
        }
        // The lead and its agents, and the plan, from the board's roles section.
        if snap["roles"].is_object() {
            let list = agent_list(&snap["roles"]);
            grant_agents(kernel, &list);
            out["agents"] = json!(list);
            out["plan"] = plan_of(&snap["roles"]);
        }
    }
    out
}

/// The team's own services the page lists under "Team" and opens in the right pane:
/// the dispatcher itself, the board, the queue (the board's store) and each github
/// connector. Only agents that exist are listed.
pub fn services_of(kernel: &fantastic_kernel::Kernel, me: &str, board: &str) -> Vec<Value> {
    let live = |id: &str| {
        kernel
            .agents
            .contains_key(&fantastic_kernel::AgentId::from(id))
    };
    let mut out = vec![json!({"id": me, "kind": "dispatcher", "label": "Dispatcher"})];
    if live(board) {
        out.push(json!({"id": board, "kind": "board", "label": "Board"}));
        let store = kernel
            .agents
            .get(&fantastic_kernel::AgentId::from(board))
            .and_then(|e| {
                e.meta
                    .read()
                    .ok()
                    .and_then(|m| m.get("store").and_then(Value::as_str).map(str::to_string))
            });
        if let Some(q) = store.filter(|q| live(q)) {
            out.push(json!({"id": q, "kind": "queue", "label": "Queue"}));
        }
    }
    let mut gh: Vec<String> = kernel
        .agents
        .iter()
        .filter(|e| {
            e.value()
                .handler_module
                .as_deref()
                .and_then(crate::agent_views::service_kind)
                == Some("github")
        })
        .map(|e| e.value().id.0.clone())
        .collect();
    gh.sort();
    for id in gh {
        out.push(json!({"id": id, "kind": "github", "label": format!("GitHub ({id})")}));
    }
    // The kinds a declaration draws (`declared_views`): cadencer, puller
    // and git, in that order, each by id.
    let declared = crate::declared_views::declared_agents(kernel);
    for kind in crate::declared_views::WEB_KINDS {
        for (id, d) in declared.iter().filter(|(_, d)| d.kind == kind) {
            out.push(json!({"id": id.0, "kind": kind, "label": format!("{} ({})", d.title, id.0)}));
        }
    }
    out
}

/// The panel reply for a service node: the model drawn in the right pane. A service
/// that errors gives a model with one "unavailable" row, never the raw error.
#[allow(clippy::too_many_arguments)]
async fn panel_reply(
    kernel: &Arc<fantastic_kernel::Kernel>,
    peers: &KernelPeers,
    meta: &serde_json::Map<String, Value>,
    board: &str,
    team: &str,
    groups: &BTreeMap<String, Vec<String>>,
    limit: u64,
    now: u64,
    svc: &Value,
) -> Value {
    use crate::agent_views as v;
    let node = svc["id"].as_str().unwrap_or("");
    let kind = svc["kind"].as_str().unwrap_or("");
    let ok = |r: &Value| r.is_object() && r.get("error").is_none();
    let model = match kind {
        "queue" | "board" => {
            let snap = peers.ask(board, json!({"type": "board"})).await;
            if !ok(&snap) {
                v::unavailable(kind, svc["label"].as_str().unwrap_or(kind))
            } else if kind == "queue" {
                v::queue_view(&snap, now)
            } else {
                v::board_view(&snap)
            }
        }
        "github" => {
            let r = peers.ask(node, json!({"type": "reflect"})).await;
            if ok(&r) {
                v::github_view(&r)
            } else {
                v::unavailable(kind, "GitHub")
            }
        }
        "dispatcher" => {
            let sb = sidebar_reply(
                kernel,
                peers,
                meta,
                board,
                team,
                groups,
                limit,
                now,
                &None,
            )
            .await;
            v::dispatcher_view(&sb)
        }
        other => match crate::declared_views::decl_by_kind(other) {
            Some(d) if crate::declared_views::WEB_KINDS.contains(&other) => {
                // A declared kind: ask each section's read verb, draw the one model.
                let mut replies = Vec::new();
                for s in &d.sections {
                    let mut ask = s.args.clone().unwrap_or_else(|| json!({}));
                    ask["type"] = json!(s.verb);
                    replies.push(peers.ask(node, ask).await);
                }
                crate::declared_views::model_of(&d, &replies)
            }
            _ => v::unavailable(other, other),
        },
    };
    json!({"view": "panel", "kind": kind, "target": node, "model": model})
}

/// The dispatcher bundle: `sidebar`, `tree`, `open`, `control`.
#[derive(Default)]
pub struct DispatcherBundle;

#[async_trait::async_trait]
impl fantastic_kernel::bundle::Handler for DispatcherBundle {
    type Call = verbs::Call;

    fn name(&self) -> &str {
        HANDLER_MODULE
    }

    fn man(&self) -> &'static str {
        MAN
    }

    fn verbs(&self) -> &'static [fantastic_kernel::Verb] {
        verbs::VERBS
    }

    fn dynamic_verbs(&self, agent: &fantastic_kernel::Agent) -> Vec<fantastic_kernel::DynVerb> {
        verbs::dynamic_verbs(agent)
    }

    fn record(&self) -> &'static fantastic_kernel::RecordDecl {
        &record::RECORD
    }

    async fn handle(
        &self,
        agent_id: &fantastic_kernel::AgentId,
        call: Self::Call,
        kernel: &Arc<fantastic_kernel::Kernel>,
    ) -> Result<fantastic_kernel::bundle::Reply, fantastic_kernel::bundle::BundleError> {
        if let verbs::Call::Boot(_) = call {
            crate::declared_views::grant_views(kernel);
            return Ok(None);
        }
        let meta = kernel
            .agents
            .get(agent_id)
            .map(|e| e.meta.read().expect("meta").clone())
            .unwrap_or_default();
        let text = |k: &str| {
            record::RECORD
                .get(&meta, k)
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default()
        };
        let board = text("board");
        let team = text("team");
        let limit = record::RECORD
            .get(&meta, "stall_minutes")
            .and_then(|v| v.as_u64())
            .unwrap_or_default();
        let groups = groups_of(&meta);
        let peers = KernelPeers(kernel.clone());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let out = match call {
            verbs::Call::Boot(_) => Value::Null,
            verbs::Call::Reflect(_) => json!({"id": agent_id.as_str(), "sentence": MAN}),
            verbs::Call::Tree(a) => {
                let node = a.node.unwrap_or_else(|| team.clone());
                let rows = rows_of(&peers, &board, &groups, limit, now).await;
                subtree(&team, &groups, &rows, &node)
            }
            verbs::Call::Sidebar(a) => {
                let mut out = sidebar_reply(
                    kernel, &peers, &meta, &board, &team, &groups, limit, now, &a.expanded,
                )
                .await;
                out["services"] = json!(services_of(kernel, agent_id.as_str(), &board));
                out
            }
            verbs::Call::Open(a) => {
                let node = a.node;
                let services = services_of(kernel, agent_id.as_str(), &board);
                if let Some(svc) = services.iter().find(|s| s["id"] == node.as_str()) {
                    let out = panel_reply(
                        kernel, &peers, &meta, &board, &team, &groups, limit, now, svc,
                    )
                    .await;
                    return Ok(Some(out));
                }
                let kind = kind_of(&node, &team, &groups);
                // A spawned lead or child is chatted with like a worker: grant it now.
                // Only an id the board's roles tree names is granted, never an arbitrary one.
                if kind == "worker" && meta.get("roles").is_some() {
                    let snap = peers.ask(&board, json!({"type": "board"})).await;
                    let list = agent_list(&snap["roles"]);
                    if list.iter().any(|a| a["id"] == node.as_str()) {
                        grant_agents(kernel, &list);
                    }
                }
                let mut out = open_target(&node, kind);
                out["kind"] = json!(kind);
                if kind == "worker" {
                    // Cards the chat shows are callable by the page's leg, and only those.
                    crate::tree::grant_cards(kernel, crate::page::WS_ID, &node).await;
                }
                out
            }
            verbs::Call::Control(a) => {
                let node = a.node.clone();
                let action = a.action;
                let reply = control_reply(&node, action, a.confirm);
                if reply["sent"] != json!(true) {
                    return Ok(Some(reply));
                }
                match action {
                    verbs::ControlAction::HandTask => {
                        let task = a.task.as_deref().unwrap_or("").trim().to_string();
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
                        relayed(reply, &node, action.as_str(), peer)
                    }
                    verbs::ControlAction::AddWorker => {
                        let at = Membership {
                            me: agent_id.as_str(),
                            roles: meta.get("roles").and_then(Value::as_str),
                            groups: &groups,
                        };
                        let out =
                            add_worker(kernel, &peers, &at, &node, (&a.id, &a.template), reply)
                                .await;
                        // The new worker may be steered by the dispatcher at once,
                        // like the workers granted at the start (`grant_control`).
                        if out.get("error").is_none() {
                            if let Some(id) = out["id"].as_str() {
                                crate::tree::grant_control(kernel, &[id.to_string()]);
                            }
                        }
                        out
                    }
                    verbs::ControlAction::RemoveWorker => {
                        let at = Membership {
                            me: agent_id.as_str(),
                            roles: meta.get("roles").and_then(Value::as_str),
                            groups: &groups,
                        };
                        remove_worker(&peers, &at, &node, reply).await
                    }
                    _ => {
                        // The node's own answer decides: a refusal or no reply is not "sent".
                        let peer = peers.ask(&node, json!({"type": action.as_str()})).await;
                        relayed(reply, &node, action.as_str(), peer)
                    }
                }
            }
        };
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_boot::{boot_at, Booted, Seed};

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
        let r = control_reply("w1", verbs::ControlAction::Stop, false);
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
        use fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
        use fantastic_kernel::Item;
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = boot_at(dir.path(), Seed::Stock, &[], None).await;
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
        kernel.stop().await;
    }

    #[test]
    fn control_sends_start_and_pause_and_stop_with_confirm() {
        for action in [verbs::ControlAction::Start, verbs::ControlAction::Pause] {
            assert_eq!(control_reply("w1", action, false)["sent"], json!(true));
        }
        let stop = control_reply("w1", verbs::ControlAction::Stop, true);
        assert_eq!(stop["sent"], json!(true), "{stop}");
        assert_eq!(stop["action"], json!("stop"));
    }

    const PROBE: &str = "probe_test.tools";

    type Seen = Arc<std::sync::Mutex<Vec<Value>>>;

    /// What each folder's probe has been sent, by the kernel's work dir (an `Extra` is a
    /// plain fn and carries no state).
    static SEEN: std::sync::Mutex<BTreeMap<std::path::PathBuf, Seen>> =
        std::sync::Mutex::new(BTreeMap::new());

    fn register_probe(reg: &mut fantastic_kernel::BundleRegistry) {
        reg.register(PROBE, ProbeBundle);
    }

    /// Records every message it gets. It refuses `pause` as an unknown verb.
    struct ProbeBundle;

    fantastic_kernel::stand_in_verbs!(probe_verbs { StartV "start" {}, PauseV "pause" {}, StopV "stop" {}, SendV "send" { text: Option<String> } });

    #[async_trait::async_trait]
    impl fantastic_kernel::bundle::Handler for ProbeBundle {
        stand_in_body!(probe_verbs);

        fn name(&self) -> &str {
            PROBE
        }

        async fn handle(
            &self,
            _agent_id: &fantastic_kernel::AgentId,
            call: Self::Call,
            kernel: &Arc<fantastic_kernel::Kernel>,
        ) -> Result<fantastic_kernel::bundle::Reply, fantastic_kernel::bundle::BundleError>
        {
            let payload = &call.payload();
            if payload["type"] == json!("boot") {
                return Ok(None);
            }
            let seen = SEEN
                .lock()
                .unwrap()
                .get(&kernel.work_dir().unwrap())
                .cloned();
            if let Some(seen) = seen {
                seen.lock().unwrap().push(payload.clone());
            }
            if payload["type"] == json!("pause") {
                return Ok(Some(json!({"error": "no such verb"})));
            }
            Ok(Some(json!({"ok": true})))
        }
    }

    /// A kernel booted with the probe registered, a dispatcher that may call it, and the
    /// probe that may be called by it.
    async fn kernel_with_probe(dir: &std::path::Path, seen: Seen) -> Booted {
        use fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
        use fantastic_kernel::Item;
        SEEN.lock().unwrap().insert(dir.to_path_buf(), seen);
        let kernel = boot_at(dir, Seed::Stock, &[register_probe], None).await;
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
        let dir = tempfile::TempDir::new().unwrap();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let kernel = kernel_with_probe(dir.path(), seen.clone()).await;
        let out = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "control", "node": "probe", "action": "frobnicate"}),
        )
        .await;
        assert_eq!(out["reason"], json!("bad_arg"), "{out}");
        assert!(
            seen.lock().unwrap().is_empty(),
            "{:?}",
            seen.lock().unwrap()
        );
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn control_via_kernel_reports_a_peer_refusal() {
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
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn control_via_kernel_stop_needs_confirm_then_reaches_the_node() {
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
        kernel.stop().await;
    }

    fn load_full() -> Value {
        json!({
            "workers": {"busy": 1, "idle": 2, "stalled": 2},
            "tasks": {"held": 1, "in_review": 3, "done": 2, "blocked": 0, "queued": 3},
            "done_last_hour": 2,
            "digest_bytes_per_hour": 480,
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
    fn sidebar_load_carries_digest_bytes_per_hour() {
        let sb = sidebar_of(&rows_w(), &load_full());
        assert_eq!(sb["load"]["digest_bytes_per_hour"], 480);
        let mut bare = load_full();
        bare.as_object_mut()
            .unwrap()
            .remove("digest_bytes_per_hour");
        let sb = sidebar_of(&rows_w(), &bare);
        assert!(sb["load"].get("digest_bytes_per_hour").is_none(), "{sb}");
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
    fn control_accepts_hand_task_add_and_remove_worker() {
        for action in [
            verbs::ControlAction::HandTask,
            verbs::ControlAction::AddWorker,
        ] {
            assert_eq!(
                control_reply("w1", action, false)["sent"],
                json!(true),
                "{action:?}"
            );
        }
        let r = control_reply("w1", verbs::ControlAction::RemoveWorker, true);
        assert_eq!(r["sent"], json!(true), "{r}");
        assert_eq!(r["action"], "remove_worker");
    }

    #[test]
    fn remove_worker_without_confirm_asks_first() {
        let r = control_reply("w1", verbs::ControlAction::RemoveWorker, false);
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

    #[tokio::test(flavor = "multi_thread")]
    async fn hand_task_needs_a_task_text() {
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
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn control_via_kernel_hand_task_reaches_the_worker() {
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
        kernel.stop().await;
    }

    /// A kernel with a dispatcher (the agent verbs on the root, `roles` naming the
    /// registry), a roles registry whose `tiers` field holds the tier `t`, a board record,
    /// the stock tree's store `memory`, and one group `g` with `w1`.
    async fn kernel_with_groups(dir: &std::path::Path) -> Booted {
        use fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
        use fantastic_kernel::Item;
        let kernel = boot_at(dir, Seed::Stock, &[], None).await;
        let can_call = json!({ (DEFAULT_ROOT_ID): crate::tree::DISPATCHER_KERNEL_VERBS });
        for item in [
            json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "dispatcher",
                "handler_module": HANDLER_MODULE, "can_call": can_call,
                "roles": "roles", "groups": {"g": ["w1"]}}),
            json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "board",
                "handler_module": "ram_store.tools"}),
            json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "roles",
                "handler_module": "agentic_resources.tools",
                "tiers": {"t": {"handler_module": "ai_chat.tools", "model": "m",
                    "provider_id": "prov", "memory_store_id": "memory"}}}),
        ] {
            let o = kernel.send(vec![Item::new(DEFAULT_ROOT_ID, item)]).await;
            assert!(o[0].result.get("error").is_none(), "{:?}", o[0].result);
        }
        // the stock tree's own `memory` store: the lead and `w1` may call it
        kernel
            .edit_callable_by(&fantastic_kernel::AgentId::from("memory"), |cb| {
                cb.insert("lead".into(), json!("*"));
                cb.insert("w1".into(), json!("*"));
                true
            })
            .unwrap();
        // the root's end of the dispatcher's `can_call` on it (an operator write)
        kernel
            .edit_callable_by(&fantastic_kernel::AgentId::from(DEFAULT_ROOT_ID), |cb| {
                cb.insert(
                    "dispatcher".into(),
                    json!(crate::tree::DISPATCHER_KERNEL_VERBS),
                );
                true
            })
            .unwrap();
        kernel
    }

    fn meta_of(kernel: &Arc<fantastic_kernel::Kernel>, id: &str) -> serde_json::Map<String, Value> {
        kernel
            .agents
            .get(&fantastic_kernel::AgentId::from(id))
            .map(|a| a.meta.read().unwrap().clone())
            .unwrap_or_default()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn control_via_kernel_add_worker_creates_the_agent_and_lists_it_in_the_tree() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_with_groups(dir.path()).await;
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
        assert_eq!(out["sent"], json!(true), "{out}");
        assert_eq!(bad["sent"], json!(false), "{bad}");
        assert!(bad["error"].as_str().unwrap().contains("no tier"), "{bad}");
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
        let tree = ask_kernel(&kernel, "dispatcher", json!({"type": "tree"})).await;
        let ids: Vec<&str> = tree["children"][0]["children"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["row"]["id"].as_str().unwrap())
            .collect();
        assert!(ids.contains(&"w2"), "{tree}");
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn control_via_kernel_add_worker_grants_the_dispatcher_the_new_worker() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_with_groups(dir.path()).await;
        let added = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "control", "node": "g", "action": "add_worker", "id": "w2", "template": "t"}),
        )
        .await;
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
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_worker_uses_the_shared_store_and_makes_no_memory() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_with_groups(dir.path()).await;
        let out = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "control", "node": "g", "action": "add_worker", "id": "w2", "template": "t"}),
        )
        .await;
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
        // the record the tier and the dispatcher's start grant make: no `*` for anyone
        assert_eq!(w["callable_by"]["$self"], json!("*"));
        assert_eq!(w["callable_by"]["$parent"], json!("*"));
        assert_eq!(
            w["callable_by"]["dispatcher"],
            json!(crate::tree::CONTROL_VERBS)
        );
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_worker_without_the_store_is_refused_and_leaves_no_worker() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_with_groups(dir.path()).await;
        let gone = ask_kernel(
            &kernel,
            "kernel_state",
            json!({"type": "delete_agent", "id": "memory"}),
        )
        .await;
        assert!(gone.get("error").is_none(), "{gone}");
        let out = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "control", "node": "g", "action": "add_worker", "id": "w2", "template": "t"}),
        )
        .await;
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
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn remove_worker_keeps_the_store() {
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
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn control_via_kernel_remove_worker_deletes_it_and_drops_it_from_the_groups() {
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
        kernel.stop().await;
    }

    fn register_load(reg: &mut fantastic_kernel::BundleRegistry) {
        reg.register("load_test.tools", LoadBundle);
    }

    /// A sidebar answers with the cadencer's load (cost per hour) when it replies.
    struct LoadBundle;

    fantastic_kernel::stand_in_verbs!(load_verbs { StatusV "status" {} });

    #[async_trait::async_trait]
    impl fantastic_kernel::bundle::Handler for LoadBundle {
        stand_in_body!(load_verbs);

        fn name(&self) -> &str {
            "load_test.tools"
        }

        async fn handle(
            &self,
            _agent_id: &fantastic_kernel::AgentId,
            call: Self::Call,
            _kernel: &Arc<fantastic_kernel::Kernel>,
        ) -> Result<fantastic_kernel::bundle::Reply, fantastic_kernel::bundle::BundleError>
        {
            let payload = &call.payload();
            Ok(match payload["type"].as_str() {
                Some("status") => Some(json!({"load": {
                    "tasks": {"in_review": 2}, "workers": {"stalled": 1},
                    "digest_bytes_per_hour": 99}})),
                _ => None,
            })
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sidebar_via_kernel_takes_the_cost_per_hour_from_the_cadencer() {
        use fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
        use fantastic_kernel::Item;
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = boot_at(dir.path(), Seed::Stock, &[register_load], None).await;
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
        assert_eq!(sb["waiting"], json!({"prs": 2, "stalled": 1}), "{sb}");
        kernel.stop().await;
    }

    /// A registry stand-in: answers `get_count` with {running: 2, max: 3}, or with a
    /// refusal when its `fail` field is set.
    struct CountBundle;

    fn register_count(reg: &mut fantastic_kernel::BundleRegistry) {
        reg.register("count_test.tools", CountBundle);
    }

    fantastic_kernel::stand_in_verbs!(count_verbs { GetCountV "get_count" { role: Option<String> } });

    #[async_trait::async_trait]
    impl fantastic_kernel::bundle::Handler for CountBundle {
        stand_in_body!(count_verbs);

        fn name(&self) -> &str {
            "count_test.tools"
        }

        async fn handle(
            &self,
            agent_id: &fantastic_kernel::AgentId,
            call: Self::Call,
            kernel: &Arc<fantastic_kernel::Kernel>,
        ) -> Result<fantastic_kernel::bundle::Reply, fantastic_kernel::bundle::BundleError>
        {
            let payload = &call.payload();
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
    ) -> Booted {
        use fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
        use fantastic_kernel::Item;
        let kernel = boot_at(dir, Seed::Stock, &[register_count], None).await;
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
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sidebar_has_no_fleet_without_a_registry() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_with_registry(dir.path(), None, false).await;
        let sb = ask_kernel(&kernel, "dispatcher", json!({"type": "sidebar"})).await;
        assert!(sb.get("fleet").is_none(), "{sb}");
        assert!(sb["team"].is_string(), "{sb}");
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sidebar_has_no_fleet_when_the_registry_refuses() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_with_registry(dir.path(), Some("reg"), true).await;
        let sb = ask_kernel(&kernel, "dispatcher", json!({"type": "sidebar"})).await;
        assert!(sb.get("fleet").is_none(), "{sb}");
        kernel.stop().await;
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

    // ---- the page's chat surface, run in node against a stub DOM ----

    // ---- roles mode: the agents list and the plan (#185) ----

    fn node(id: &str, role: &str, state: &str, kids: Vec<Value>) -> Value {
        json!({"id": id, "role": role, "state": state, "children": kids})
    }

    fn ids(list: &[Value]) -> Vec<String> {
        list.iter()
            .map(|a| a["id"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn agent_list_puts_the_lead_first() {
        // The lead sits in the middle of the fleet: it is still item 0.
        let roles = json!({"lead": "pl", "tree": node("root", "", "active", vec![
            node("w0", "worker", "active", vec![]),
            node("pl", "project_lead", "active", vec![node("w1", "worker", "active", vec![])]),
            node("w2", "worker", "done", vec![]),
        ])});
        let list = agent_list(&roles);
        assert_eq!(list[0]["id"], "pl", "{list:?}");
        assert_eq!(list[0]["lead"], true);
        assert_eq!(list[0]["label"], "Lead");
        assert_eq!(list.iter().filter(|a| a["lead"] == true).count(), 1);
    }

    #[test]
    fn agent_list_nests_children_depth_first_with_roles_and_states() {
        let roles = json!({"lead": "pl", "tree": node("pl", "project_lead", "active", vec![
            node("sub", "project_lead", "paused", vec![node("w1", "worker", "done", vec![])]),
            node("w2", "worker", "active", vec![]),
        ])});
        let list = agent_list(&roles);
        assert_eq!(ids(&list), ["pl", "sub", "w1", "w2"]);
        let depth: Vec<u64> = list.iter().map(|a| a["depth"].as_u64().unwrap()).collect();
        assert_eq!(depth, [0, 1, 2, 1]);
        let state: Vec<&str> = list.iter().map(|a| a["state"].as_str().unwrap()).collect();
        assert_eq!(state, ["active", "paused", "done", "active"]);
        assert_eq!(list[2]["role"], "worker");
        assert_eq!(list[2]["label"], "worker");
        assert_eq!(list[2]["indent"], 2);
        assert_eq!(list[2]["guides"], 2);
        assert_eq!(list[2]["lead"], false);
    }

    #[test]
    fn agent_list_labels_a_roleless_agent_by_its_id_and_defaults_the_state() {
        let roles = json!({"lead": "pl", "tree": {"id": "pl", "children": [{"id": "x1"}]}});
        let list = agent_list(&roles);
        assert_eq!(list[1]["label"], "x1");
        assert_eq!(list[1]["state"], "active");
        assert_eq!(list[0]["role"], "project_lead");
    }

    #[test]
    fn agent_list_caps_the_indent_but_keeps_depth() {
        let mut tree = node("n7", "worker", "active", vec![]);
        for i in (0..7).rev() {
            tree = node(&format!("n{i}"), "project_lead", "active", vec![tree]);
        }
        let list = agent_list(&json!({"lead": "n0", "tree": tree}));
        assert_eq!(list.len(), 8);
        for (d, a) in list.iter().enumerate() {
            assert_eq!(a["depth"], d, "{a}");
            assert_eq!(a["indent"], d.min(INDENT_CAP), "{a}");
            assert_eq!(a["guides"], a["indent"]);
        }
        assert_eq!(list[7]["indent"], INDENT_CAP);
        assert_eq!(list[7]["depth"], 7);
    }

    #[test]
    fn agent_list_with_only_a_lead_id_and_no_tree() {
        let list = agent_list(&json!({"lead": "pl", "tree": null}));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["id"], "pl");
        assert_eq!(list[0]["lead"], true);
        assert_eq!(list[0]["state"], "active");
        assert_eq!(list[0]["role"], "project_lead");
        assert!(agent_list(&json!({"lead": null, "tree": null})).is_empty());
    }

    #[test]
    fn agent_list_empty_without_roles_mode() {
        assert!(agent_list(&Value::Null).is_empty());
        assert!(agent_list(&json!({})).is_empty());
    }

    #[test]
    fn plan_of_carries_tasks_holder_and_fleet() {
        let roles = json!({"lead": "pl", "fleet": {"running": 2, "max": 5}, "tasks": [
            {"id": "o/r#1", "source": "gh:o/r#1", "holder": "w1", "status": "in_work", "extra": 1},
            {"id": "o/r#2", "source": "gh:o/r#2", "holder": {"subtree": "sub"}, "status": "open"},
        ]});
        let plan = plan_of(&roles);
        assert_eq!(plan["lead"], "pl");
        assert_eq!(plan["fleet"], json!({"running": 2, "max": 5}));
        assert_eq!(
            plan["tasks"][0],
            json!({"id": "o/r#1", "source": "gh:o/r#1", "holder": "w1", "status": "in_work"})
        );
        assert_eq!(plan["tasks"][1]["holder"], "subtree:sub");
        assert_eq!(plan_of(&Value::Null)["tasks"], json!([]));
    }

    /// A board stand-in: answers `board` with the `roles` section its meta holds (when any).
    struct RolesBoard;

    fn register_roles_mocks(reg: &mut fantastic_kernel::BundleRegistry) {
        reg.register("count_test.tools", CountBundle);
        reg.register("roles_board_test.tools", RolesBoard);
    }

    fantastic_kernel::stand_in_verbs!(roles_board_verbs { BoardV "board" {} });

    #[async_trait::async_trait]
    impl fantastic_kernel::bundle::Handler for RolesBoard {
        stand_in_body!(roles_board_verbs);

        fn name(&self) -> &str {
            "roles_board_test.tools"
        }

        async fn handle(
            &self,
            agent_id: &fantastic_kernel::AgentId,
            call: Self::Call,
            kernel: &Arc<fantastic_kernel::Kernel>,
        ) -> Result<fantastic_kernel::bundle::Reply, fantastic_kernel::bundle::BundleError>
        {
            let payload = &call.payload();
            if payload["type"] != "board" {
                return Ok(None);
            }
            let mut out = json!({"tasks": {"in_work": [], "done": [], "pending": [], "failed": []}, "agents": {}});
            if let Some(r) = kernel
                .agents
                .get(agent_id)
                .and_then(|a| a.meta.read().unwrap().get("roles_section").cloned())
            {
                out["roles"] = r;
            }
            Ok(Some(out))
        }
    }

    /// A kernel with a dispatcher (roles mode when `section` is given), a board stand-in,
    /// the `team_ws` leg, and the agents `pl` (the lead) and `c1` (its child), made after
    /// the start so nobody granted them a chat.
    async fn kernel_in_roles_mode(dir: &std::path::Path, section: Option<Value>) -> Booted {
        use fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
        use fantastic_kernel::Item;
        let kernel = boot_at(dir, Seed::Stock, &[register_roles_mocks], None).await;
        let mut d = json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "dispatcher",
            "handler_module": HANDLER_MODULE, "can_call": {"reg": "*", "board": "*"}});
        let mut b = json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "board",
            "handler_module": "roles_board_test.tools", "callable_by": {"dispatcher": "*"}});
        if let Some(s) = section {
            d["roles"] = json!("reg");
            b["roles_section"] = s;
        }
        let reg = json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "reg",
            "handler_module": "count_test.tools", "callable_by": {"dispatcher": "*"}});
        let ws = json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "team_ws",
            "handler_module": "count_test.tools"});
        let pl = json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "pl",
            "handler_module": "count_test.tools"});
        let c1 = json!({"type": "create_agent", "parent_id": "pl", "id": "c1",
            "handler_module": "count_test.tools"});
        for item in [d, b, reg, ws, pl, c1] {
            kernel.send(vec![Item::new(DEFAULT_ROOT_ID, item)]).await;
        }
        kernel
    }

    fn lead_section() -> Value {
        json!({"lead": "pl", "fleet": {"running": 2, "max": 3},
            "tree": node("pl", "project_lead", "active", vec![node("c1", "worker", "paused", vec![])]),
            "agents": [{"id": "c1", "role": "worker", "state": "paused", "parent": "pl"}],
            "tasks": [{"id": "o/r#1", "source": "gh:o/r#1", "holder": "c1", "status": "in_work"}]})
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sidebar_in_roles_mode_carries_agents_and_plan() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_in_roles_mode(dir.path(), Some(lead_section())).await;
        let sb = ask_kernel(&kernel, "dispatcher", json!({"type": "sidebar"})).await;
        assert_eq!(sb["agents"][0]["id"], "pl", "{sb}");
        assert_eq!(sb["agents"][0]["label"], "Lead");
        assert_eq!(sb["agents"][1]["id"], "c1");
        assert_eq!(sb["plan"]["tasks"][0]["id"], "o/r#1", "{sb}");
        assert_eq!(sb["plan"]["lead"], "pl");
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sidebar_without_roles_mode_has_neither_agents_nor_plan() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_in_roles_mode(dir.path(), None).await;
        let sb = ask_kernel(&kernel, "dispatcher", json!({"type": "sidebar"})).await;
        assert!(sb.get("agents").is_none(), "{sb}");
        assert!(sb.get("plan").is_none(), "{sb}");
        assert!(sb["team"].is_string());
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sidebar_in_roles_mode_grants_chat_to_the_lead_and_its_children() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_in_roles_mode(dir.path(), Some(lead_section())).await;
        let chat = json!(crate::tree::CHAT_VERBS);
        let before = meta_of(&kernel, "team_ws");
        assert!(before.get("can_call").is_none_or(|c| c.get("pl").is_none()));
        ask_kernel(&kernel, "dispatcher", json!({"type": "sidebar"})).await;
        let ws = meta_of(&kernel, "team_ws");
        for id in ["pl", "c1"] {
            assert_eq!(ws["can_call"][id], chat, "team_ws -> {id}");
            let m = meta_of(&kernel, id);
            assert_eq!(m["callable_by"]["team_ws"], chat, "{id}");
            assert_eq!(m["callable_by"]["dispatcher_view"], json!(null));
        }
        assert!(ws["can_call"].get("*").is_none());
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn open_of_a_roles_agent_returns_the_chat_view_and_grants_only_known_ids() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_in_roles_mode(dir.path(), Some(lead_section())).await;
        let out = ask_kernel(&kernel, "dispatcher", json!({"type": "open", "node": "pl"})).await;
        assert_eq!(out["view"], "chat", "{out}");
        assert_eq!(out["target"], "pl");
        let ws = meta_of(&kernel, "team_ws");
        assert_eq!(ws["can_call"]["pl"], json!(crate::tree::CHAT_VERBS));
        // An id the roles tree does not name gets no grant.
        ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "open", "node": "reg"}),
        )
        .await;
        let ws = meta_of(&kernel, "team_ws");
        assert!(ws["can_call"].get("reg").is_none(), "{ws:?}");
        kernel.stop().await;
    }

    /// A board stand-in for the service views: answers `board` with the `board_reply` its
    /// meta holds, or with an error carrying a secret when its `fail` field is set.
    struct ViewsBoard;

    fn register_services_mocks(reg: &mut fantastic_kernel::BundleRegistry) {
        reg.register("views_board_test.tools", ViewsBoard);
        reg.register("team_github.tools", StubGithub);
        reg.register("count_test.tools", CountBundle);
    }

    fantastic_kernel::stand_in_verbs!(views_board_verbs { BoardV "board" {} });

    #[async_trait::async_trait]
    impl fantastic_kernel::bundle::Handler for ViewsBoard {
        stand_in_body!(views_board_verbs);

        fn name(&self) -> &str {
            "views_board_test.tools"
        }

        async fn handle(
            &self,
            agent_id: &fantastic_kernel::AgentId,
            call: Self::Call,
            kernel: &Arc<fantastic_kernel::Kernel>,
        ) -> Result<fantastic_kernel::bundle::Reply, fantastic_kernel::bundle::BundleError>
        {
            let payload = &call.payload();
            if payload["type"] != "board" {
                return Ok(None);
            }
            let m = kernel
                .agents
                .get(agent_id)
                .map(|a| a.meta.read().unwrap().clone())
                .unwrap_or_default();
            if m.contains_key("fail") {
                return Ok(Some(json!({"error": format!("boom {}", secret_text())})));
            }
            Ok(Some(m.get("board_reply").cloned().unwrap_or(json!({}))))
        }
    }

    /// A github connector stand-in: answers `reflect` with the `reflect` its meta holds.
    struct StubGithub;

    mod gh_verbs {
        fantastic_kernel::verbs!();
    }

    #[async_trait::async_trait]
    impl fantastic_kernel::bundle::Handler for StubGithub {
        stand_in_body!(gh_verbs);

        fn name(&self) -> &str {
            "team_github.tools"
        }

        async fn handle(
            &self,
            agent_id: &fantastic_kernel::AgentId,
            call: Self::Call,
            kernel: &Arc<fantastic_kernel::Kernel>,
        ) -> Result<fantastic_kernel::bundle::Reply, fantastic_kernel::bundle::BundleError>
        {
            if !matches!(call, gh_verbs::Call::Reflect(_)) {
                return Ok(None);
            }
            let m = kernel
                .agents
                .get(agent_id)
                .map(|a| a.meta.read().unwrap().clone())
                .unwrap_or_default();
            if m.contains_key("fail") {
                return Ok(Some(json!({"error": format!("boom {}", secret_text())})));
            }
            Ok(Some(m.get("reflect").cloned().unwrap_or(json!({}))))
        }
    }

    /// A token-shaped text, built from pieces so no literal sits in the source.
    fn secret_text() -> String {
        format!("{}{}{}", "gh", "s_", "Zq9x".repeat(9))
    }

    /// A kernel with a dispatcher, a board stand-in (store `queue`), the queue, and
    /// (when `gh`) a github stand-in; `fail` makes the board and github stand-ins error.
    async fn kernel_with_services(
        dir: &std::path::Path,
        board_reply: Value,
        gh: bool,
        fail: bool,
    ) -> Booted {
        use fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
        use fantastic_kernel::Item;
        let kernel = boot_at(dir, Seed::Stock, &[register_services_mocks], None).await;
        let mut items = vec![
            json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "dispatcher",
                "handler_module": HANDLER_MODULE,
                "can_call": {"board": "*", "github": "*", "queue": "*"}}),
            json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "board",
                "handler_module": "views_board_test.tools", "store": "queue",
                "board_reply": board_reply, "callable_by": {"dispatcher": "*"}}),
            json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "queue",
                "handler_module": "count_test.tools"}),
        ];
        if fail {
            items[1]["fail"] = json!(true);
        }
        if gh {
            let mut g = json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "github",
                "handler_module": "team_github.tools", "callable_by": {"dispatcher": ["reflect"]},
                "reflect": {
                    "repos": ["acme/widgets"], "api_url": "https://api.github.com",
                    "recent_calls": [{"verb": "issue_view", "target": "acme/widgets#3", "result": "ok"}],
                    "rate": {"limit": 5000, "remaining": 4990, "reset": 1760000000u64},
                    "token": secret_text(), "private_key": format!("-----BEGIN {} KEY-----", "RSA PRIVATE"),
                    "authorization": format!("Bearer {}", secret_text())}});
            if fail {
                g["fail"] = json!(true);
            }
            items.push(g);
        }
        for item in items {
            kernel.send(vec![Item::new(DEFAULT_ROOT_ID, item)]).await;
        }
        kernel
    }

    fn ids_of(services: &Value) -> Vec<(String, String)> {
        services
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                (
                    s["id"].as_str().unwrap().to_string(),
                    s["kind"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    fn pair(a: &str, b: &str) -> (String, String) {
        (a.to_string(), b.to_string())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sidebar_carries_services_in_every_mode() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_with_services(dir.path(), json!({}), true, false).await;
        let sb = ask_kernel(&kernel, "dispatcher", json!({"type": "sidebar"})).await;
        let want = vec![
            pair("dispatcher", "dispatcher"),
            pair("board", "board"),
            pair("queue", "queue"),
            pair("github", "github"),
        ];
        assert_eq!(ids_of(&sb["services"]), want, "{sb}");
        assert!(sb.get("agents").is_none(), "this is not roles mode: {sb}");
        kernel.stop().await;

        // Roles mode too.
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_in_roles_mode(dir.path(), Some(lead_section())).await;
        let sb = ask_kernel(&kernel, "dispatcher", json!({"type": "sidebar"})).await;
        assert!(sb["agents"].is_array());
        assert_eq!(
            ids_of(&sb["services"]),
            vec![pair("dispatcher", "dispatcher"), pair("board", "board")],
            "{sb}"
        );
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sidebar_services_skip_agents_that_do_not_exist() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_with_services(dir.path(), json!({}), false, false).await;
        let sb = ask_kernel(&kernel, "dispatcher", json!({"type": "sidebar"})).await;
        assert_eq!(
            ids_of(&sb["services"]),
            vec![
                pair("dispatcher", "dispatcher"),
                pair("board", "board"),
                pair("queue", "queue")
            ],
            "no github agent, none listed: {sb}"
        );
        // A board whose store is gone lists no queue.
        ask_kernel(
            &kernel,
            fantastic_kernel::bootstrap::DEFAULT_ROOT_ID,
            json!({"type": "delete_agent", "id": "queue"}),
        )
        .await;
        let sb = ask_kernel(&kernel, "dispatcher", json!({"type": "sidebar"})).await;
        assert!(
            !ids_of(&sb["services"]).iter().any(|(_, k)| k == "queue"),
            "{sb}"
        );
        kernel.stop().await;
    }

    fn queue_board() -> Value {
        json!({"tasks": {
            "pending": [{"id": "o/r#1", "title": "first", "claimers": [], "updated": 0}],
            "in_work": [{"id": "o/r#2", "title": "second", "claimers": ["w1"], "updated": 0}],
            "done": [], "failed": []}, "agents": {"w1": {"state": "coding", "cost": 0.5}}})
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn open_of_queue_returns_panel_model_from_the_board() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_with_services(dir.path(), queue_board(), false, false).await;
        let out = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "open", "node": "queue"}),
        )
        .await;
        assert_eq!(out["view"], "panel", "{out}");
        assert_eq!(out["kind"], "queue");
        assert_eq!(out["target"], "queue");
        let m = &out["model"];
        assert_eq!(m["kind"], "queue");
        assert_eq!(m["sections"][0]["name"], "queued");
        assert_eq!(m["sections"][0]["rows"][0][0], "first");
        assert_eq!(m["sections"][1]["rows"][0][1], "w1");
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn open_of_board_in_roles_mode_returns_tree_and_tasks() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut b = queue_board();
        b["roles"] = lead_section();
        let kernel = kernel_with_services(dir.path(), b, false, false).await;
        let out = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "open", "node": "board"}),
        )
        .await;
        assert_eq!(out["view"], "panel", "{out}");
        assert_eq!(out["kind"], "board");
        let m = &out["model"];
        assert_eq!(m["sections"][0]["name"], "agents");
        assert_eq!(m["sections"][0]["rows"][0][0], "Lead");
        assert_eq!(m["sections"][1]["name"], "tasks");
        assert_eq!(
            m["sections"][1]["rows"][0],
            json!(["o/r#1", "gh:o/r#1", "c1", "in_work"])
        );
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn open_of_dispatcher_returns_its_own_workers_and_lanes() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_with_services(dir.path(), queue_board(), false, false).await;
        let out = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "open", "node": "dispatcher"}),
        )
        .await;
        assert_eq!(out["view"], "panel", "{out}");
        assert_eq!(out["kind"], "dispatcher");
        assert_eq!(out["model"]["sections"][0]["name"], "workers");
        assert_eq!(out["model"]["sections"][0]["rows"][0][0], "w1");
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn open_of_github_returns_panel_from_reflect_and_no_token() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_with_services(dir.path(), json!({}), true, false).await;
        let out = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "open", "node": "github"}),
        )
        .await;
        assert_eq!(out["view"], "panel", "{out}");
        assert_eq!(out["kind"], "github");
        let s = out.to_string();
        assert!(s.contains("acme/widgets#3"), "{s}");
        assert!(s.contains("4990/5000"), "{s}");
        assert!(!s.contains(&secret_text()), "{s}");
        assert!(!s.contains("PRIVATE"), "{s}");
        assert!(!s.contains("Bearer"), "{s}");
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn open_of_a_peer_that_errors_returns_unavailable_row() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_with_services(dir.path(), json!({}), true, true).await;
        for node in ["queue", "board", "github"] {
            let out =
                ask_kernel(&kernel, "dispatcher", json!({"type": "open", "node": node})).await;
            assert_eq!(out["view"], "panel", "{node}: {out}");
            assert_eq!(
                out["model"]["sections"][0]["rows"][0][0], "unavailable",
                "{out}"
            );
            let s = out.to_string();
            assert!(
                !s.contains(&secret_text()) && !s.contains("boom"),
                "{node}: {s}"
            );
        }
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn open_of_worker_and_group_unchanged() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_with_services(dir.path(), queue_board(), true, false).await;
        let w = ask_kernel(&kernel, "dispatcher", json!({"type": "open", "node": "w1"})).await;
        assert_eq!(w["view"], "chat", "{w}");
        assert_eq!(w["target"], "w1");
        assert_eq!(w["kind"], "worker");
        let g = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "open", "node": "team"}),
        )
        .await;
        assert_eq!(g["view"], "dispatcher", "{g}");
        assert!(g.get("model").is_none());
        kernel.stop().await;
    }

    // ---- the right pane, cards, tree and phone ----

    #[test]
    fn agent_list_marks_rows_with_children() {
        let roles = json!({"lead": "pl", "tree": node("pl", "project_lead", "active", vec![
            node("a", "worker", "active", vec![]),
            node("b", "project_lead", "active", vec![node("c", "worker", "active", vec![])]),
        ])});
        let list = agent_list(&roles);
        let flags: Vec<(&str, bool)> = list
            .iter()
            .map(|a| {
                (
                    a["id"].as_str().unwrap(),
                    a["has_children"].as_bool().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            flags,
            [("pl", true), ("a", false), ("b", true), ("c", false)]
        );
    }

    #[test]
    fn agent_list_has_children_follows_depth_not_order() {
        // Two siblings in a row: neither has children, though one follows the other.
        let roles = json!({"lead": "pl", "tree": node("pl", "project_lead", "active", vec![
            node("a", "worker", "active", vec![]),
            node("b", "worker", "active", vec![]),
        ])});
        let list = agent_list(&roles);
        assert_eq!(list[1]["has_children"], false);
        assert_eq!(list[2]["has_children"], false);
        // A deep row that is last, and a lead alone.
        let deep = json!({"lead": "pl", "tree": node("pl", "project_lead", "active", vec![
            node("s", "project_lead", "active", vec![node("t", "project_lead", "active", vec![node("u", "worker", "active", vec![])])]),
        ])});
        let flags: Vec<bool> = agent_list(&deep)
            .iter()
            .map(|a| a["has_children"].as_bool().unwrap())
            .collect();
        assert_eq!(flags, [true, true, true, false]);
        let alone = agent_list(&json!({"lead": "pl"}));
        assert_eq!(alone[0]["has_children"], false);
    }

    /// A stand-in for a team module: answers every verb with the `reply` its meta holds,
    /// or with an error carrying a secret when its `fail` field is set.
    struct StubDeclared(&'static str);

    /// The stand-ins of the three team modules the tests below declare, after the team's own.
    fn register_declared(reg: &mut fantastic_kernel::BundleRegistry) {
        for module in ["team_cadencer.tools", "team_git.tools", "team_puller.tools"] {
            reg.register(module, StubDeclared(module));
        }
    }

    fantastic_kernel::stand_in_verbs!(declared_verbs { StatusV "status" {} });

    #[async_trait::async_trait]
    impl fantastic_kernel::bundle::Handler for StubDeclared {
        stand_in_body!(declared_verbs);

        fn name(&self) -> &str {
            self.0
        }

        async fn handle(
            &self,
            agent_id: &fantastic_kernel::AgentId,
            call: Self::Call,
            kernel: &Arc<fantastic_kernel::Kernel>,
        ) -> Result<fantastic_kernel::bundle::Reply, fantastic_kernel::bundle::BundleError>
        {
            let payload = &call.payload();
            if payload["type"] == "boot" {
                return Ok(None);
            }
            let m = kernel
                .agents
                .get(agent_id)
                .map(|a| a.meta.read().unwrap().clone())
                .unwrap_or_default();
            if m.contains_key("fail") {
                return Ok(Some(json!({"error": format!("boom {}", secret_text())})));
            }
            Ok(Some(m.get("reply").cloned().unwrap_or(json!({}))))
        }
    }

    /// A kernel with a dispatcher and the given `(id, handler_module, reply)` agents, whose
    /// modules are stand-ins; the dispatcher may call each of them.
    async fn kernel_with_declared(
        dir: &std::path::Path,
        agents: &[(&str, &'static str, Value)],
    ) -> Booted {
        use fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
        use fantastic_kernel::Item;
        let kernel = boot_at(dir, Seed::Stock, &[register_declared], None).await;
        let can_call: serde_json::Map<String, Value> = agents
            .iter()
            .map(|(id, _, _)| (id.to_string(), json!("*")))
            .collect();
        kernel
            .send(vec![Item::new(
                DEFAULT_ROOT_ID,
                json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "dispatcher",
                    "handler_module": HANDLER_MODULE, "can_call": can_call}),
            )])
            .await;
        for (id, module, reply) in agents {
            kernel
                .send(vec![Item::new(
                    DEFAULT_ROOT_ID,
                    json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": id,
                        "handler_module": module, "reply": reply,
                        "callable_by": {"dispatcher": "*"}}),
                )])
                .await;
        }
        kernel
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn services_list_the_new_kinds_when_live() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_with_declared(
            dir.path(),
            &[
                ("cad", "team_cadencer.tools", json!({})),
                ("g1", "team_git.tools", json!({})),
            ],
        )
        .await;
        let sb = ask_kernel(&kernel, "dispatcher", json!({"type": "sidebar"})).await;
        assert_eq!(
            ids_of(&sb["services"]),
            vec![
                pair("dispatcher", "dispatcher"),
                pair("cad", "cadencer"),
                pair("g1", "git")
            ],
            "{sb}"
        );
        assert_eq!(sb["services"][1]["label"], "Cadencer (cad)");
        kernel.stop().await;

        // Without the agents, none is listed.
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_with_declared(dir.path(), &[]).await;
        let sb = ask_kernel(&kernel, "dispatcher", json!({"type": "sidebar"})).await;
        assert_eq!(
            ids_of(&sb["services"]),
            vec![pair("dispatcher", "dispatcher")],
            "{sb}"
        );
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn panel_reply_for_a_declared_kind_returns_the_model() {
        let status = json!({"settings": {"enabled": true, "cadence": 300, "stall_after": 1800},
            "ticks": 7, "last_beat_s": 100, "next_beat_s": 400, "digests_sent": 3,
            "load": {"workers": {"busy": 1, "idle": 0, "stalled": 2}}});
        let dir = tempfile::TempDir::new().unwrap();
        let kernel =
            kernel_with_declared(dir.path(), &[("cad", "team_cadencer.tools", status)]).await;
        let out = ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "open", "node": "cad"}),
        )
        .await;
        assert_eq!(out["view"], "panel", "{out}");
        assert_eq!(out["kind"], "cadencer");
        assert_eq!(out["target"], "cad");
        let m = &out["model"];
        assert_eq!(m["kind"], "cadencer", "{out}");
        assert_eq!(m["sections"][0]["rows"][0], json!(["on", "true"]), "{out}");
        assert_eq!(
            m["sections"][1]["rows"][0],
            json!(["stalled", "2"]),
            "{out}"
        );
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn panel_reply_for_a_refusing_declared_kind_is_unavailable_without_the_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel =
            kernel_with_declared(dir.path(), &[("wh", "team_puller.tools", json!({}))]).await;
        kernel.update(&fantastic_kernel::AgentId::from("wh"), {
            let mut m = serde_json::Map::new();
            m.insert("fail".into(), json!(true));
            m
        });
        let out = ask_kernel(&kernel, "dispatcher", json!({"type": "open", "node": "wh"})).await;
        assert_eq!(out["view"], "panel", "{out}");
        assert_eq!(out["model"]["summary"], json!(["unavailable"]), "{out}");
        assert!(!out.to_string().contains(&secret_text()), "{out}");
        kernel.stop().await;
    }
}
