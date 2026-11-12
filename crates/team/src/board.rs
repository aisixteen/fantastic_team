//! Board: the shift's shared picture of the work. It caches every open issue
//! of the configured repos (through `team_github.tools`) with its queue state
//! (`queue/*.state`), knows the agents, keeps its own state file in the store,
//! and audits the picture. It NEVER assigns: no verb sets an assignee or writes
//! a claim for someone else. Audit findings only become asks.

use std::collections::BTreeMap;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::dispatcher;
use crate::puller::{task_file_names, KernelPeers, Peers};

/// The bundle's handler module name.
pub const HANDLER_MODULE: &str = "team_board.tools";
/// What the bundle is for: its one short man text; how to call it is rendered from its
/// declared verbs (`docs`, `reflect`).
const MAN: &str = "The team's shared picture of the work: it caches the open issues of its repos with their queue state, knows the agents, keeps its own state file in the store and audits the picture. It never assigns.";

pub mod record;
pub mod verbs;
/// Failures by one agent before the audit flags it.
pub const FAIL_LIMIT: u32 = 3;
/// Where the board keeps its state inside the store.
const STATE_PATH: &str = "board/state.json";

/// Where a task stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Accepted, nobody working on it.
    #[default]
    Pending,
    /// Claimed and being worked.
    InWork,
    /// Finished, with a PR.
    Done,
    /// Given up.
    Failed,
}

impl Status {
    /// The wire name of the status.
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::InWork => "in_work",
            Status::Done => "done",
            Status::Failed => "failed",
        }
    }

    /// The status with this wire name.
    pub fn parse(s: &str) -> Option<Status> {
        match s {
            "pending" => Some(Status::Pending),
            "in_work" => Some(Status::InWork),
            "done" => Some(Status::Done),
            "failed" => Some(Status::Failed),
            _ => None,
        }
    }
}

/// One issue as the board sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// `repo#number`.
    pub id: String,
    /// The issue title.
    pub title: String,
    /// Where the task stands.
    pub status: Status,
    /// Agents named by queue state files as claimers.
    pub claimers: Vec<String>,
    /// The PR link, once done.
    pub pr: Option<String>,
    /// The last note an agent reported.
    pub note: Option<String>,
    /// Unix seconds of the last change.
    pub updated: u64,
}

/// One entry in the board's history: a report, or an ask and its answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// Unix seconds.
    pub at: u64,
    /// Who wrote it.
    pub author: String,
    /// `report` or `ask:<to>`.
    pub kind: String,
    /// The text.
    pub text: String,
    /// The answer, once it came back.
    pub answer: Option<String>,
}

/// What the board knows about one agent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentInfo {
    /// The agent's own description of its state.
    pub state: String,
    /// Unix seconds of its last report.
    pub last_seen: u64,
    /// Its cost so far, as it reports it.
    pub cost: f64,
}

/// The whole picture. Serialized into the store on every change.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Board {
    /// Tasks by `repo#number`.
    pub tasks: BTreeMap<String, Task>,
    /// Agents by id.
    pub agents: BTreeMap<String, AgentInfo>,
    /// Reports, oldest first.
    pub reports: Vec<Record>,
    /// Asks and their answers, oldest first.
    pub asks: Vec<Record>,
    /// Failed reports per agent.
    pub failures: BTreeMap<String, u32>,
    /// PR link to the task it belongs to.
    pub prs: BTreeMap<String, String>,
}

/// One thing the audit found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    /// A claim has been silent past the limit.
    StaleClaim {
        /// The task.
        task: String,
    },
    /// Work is recorded on a task nobody claimed.
    UnclaimedWork {
        /// The task.
        task: String,
    },
    /// Two agents claim one task.
    DoubleClaim {
        /// The task.
        task: String,
    },
    /// Done, but no PR is linked.
    DoneWithoutPr {
        /// The task.
        task: String,
    },
    /// A PR points at a task the board does not have.
    PrWithoutTask {
        /// The PR link.
        pr: String,
    },
    /// An agent has failed repeatedly.
    RepeatedFailure {
        /// The agent.
        agent: String,
    },
}

impl Finding {
    /// The sentence an ask carries.
    pub fn text(&self) -> String {
        match self {
            Finding::StaleClaim { task } => format!("{task}: claim has been silent past the limit"),
            Finding::UnclaimedWork { task } => {
                format!("{task}: work recorded but nobody claimed it")
            }
            Finding::DoubleClaim { task } => format!("{task}: more than one agent claims it"),
            Finding::DoneWithoutPr { task } => format!("{task}: done with no PR linked"),
            Finding::PrWithoutTask { pr } => format!("{pr}: PR points at no known task"),
            Finding::RepeatedFailure { agent } => format!("{agent}: failing repeatedly"),
        }
    }
}

/// Parse a queue state file: `pending`, `in_work <claimer>`, `done <pr>`,
/// `failed`. Returns the status, the claimer and the PR link, if written.
pub fn parse_state(content: &str) -> Option<(Status, Option<String>, Option<String>)> {
    let line = content.lines().next()?.trim();
    let mut parts = line.splitn(2, ' ');
    let status = Status::parse(parts.next()?)?;
    let rest = parts
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Some(match status {
        Status::InWork => (status, rest, None),
        Status::Done => (status, None, rest),
        _ => (status, None, None),
    })
}

impl Board {
    /// Cache an issue as pending. An issue already cached keeps its status.
    pub fn add_issue(&mut self, repo: &str, number: u64, title: &str, now: u64) {
        let id = format!("{repo}#{number}");
        self.tasks.entry(id.clone()).or_insert_with(|| Task {
            id,
            title: title.into(),
            status: Status::Pending,
            updated: now,
            ..Default::default()
        });
    }

    /// Apply a queue state file to a cached task. A claim is recorded as
    /// written by the file; nothing here writes a claim.
    pub fn apply_state(&mut self, id: &str, content: &str, now: u64) {
        let Some(task) = self.tasks.get_mut(id) else {
            return;
        };
        let Some((status, claimer, pr)) = parse_state(content) else {
            return;
        };
        task.status = status;
        if let Some(c) = claimer {
            if !task.claimers.contains(&c) {
                task.claimers.push(c);
            }
        }
        if let Some(p) = pr {
            self.prs.insert(p.clone(), id.to_string());
            task.pr = Some(p);
        }
        task.updated = now;
    }

    /// Record what an agent did or decided on a task.
    pub fn report(
        &mut self,
        id: &str,
        status: Status,
        note: &str,
        pr: Option<&str>,
        author: &str,
        now: u64,
    ) -> Result<(), String> {
        let task = self
            .tasks
            .get_mut(id)
            .ok_or_else(|| format!("no task `{id}`"))?;
        task.status = status;
        task.note = (!note.is_empty()).then(|| note.to_string());
        task.updated = now;
        if let Some(p) = pr {
            self.prs.insert(p.to_string(), id.to_string());
            task.pr = Some(p.to_string());
        }
        if status == Status::Failed {
            *self.failures.entry(author.to_string()).or_insert(0) += 1;
        }
        self.reports.push(Record {
            at: now,
            author: author.into(),
            kind: "report".into(),
            text: format!("{id} {}: {note}", status.as_str()),
            answer: None,
        });
        Ok(())
    }

    /// Record an agent's own state, cost and time.
    pub fn touch_agent(&mut self, name: &str, state: &str, cost: f64, now: u64) {
        self.agents.insert(
            name.into(),
            AgentInfo {
                state: state.into(),
                last_seen: now,
                cost,
            },
        );
    }

    /// The whole picture: tasks grouped by state, and the agents.
    pub fn snapshot(&self) -> Value {
        let mut by: BTreeMap<&str, Vec<&Task>> = BTreeMap::new();
        for s in [
            Status::Pending,
            Status::InWork,
            Status::Done,
            Status::Failed,
        ] {
            by.insert(s.as_str(), Vec::new());
        }
        for task in self.tasks.values() {
            by.entry(task.status.as_str()).or_default().push(task);
        }
        json!({"tasks": by, "agents": self.agents})
    }

    /// Run the rules over the cached state. Pure: reads only.
    pub fn audit(&self, now: u64, stale_secs: u64) -> Vec<Finding> {
        let mut out = Vec::new();
        for task in self.tasks.values() {
            if task.status == Status::InWork && now.saturating_sub(task.updated) > stale_secs {
                out.push(Finding::StaleClaim {
                    task: task.id.clone(),
                });
            }
            if matches!(task.status, Status::InWork | Status::Done) && task.claimers.is_empty() {
                out.push(Finding::UnclaimedWork {
                    task: task.id.clone(),
                });
            }
            if task.claimers.len() > 1 {
                out.push(Finding::DoubleClaim {
                    task: task.id.clone(),
                });
            }
            if task.status == Status::Done && task.pr.is_none() {
                out.push(Finding::DoneWithoutPr {
                    task: task.id.clone(),
                });
            }
        }
        for (pr, task) in &self.prs {
            if !self.tasks.contains_key(task) {
                out.push(Finding::PrWithoutTask { pr: pr.clone() });
            }
        }
        for (agent, n) in &self.failures {
            if *n >= FAIL_LIMIT {
                out.push(Finding::RepeatedFailure {
                    agent: agent.clone(),
                });
            }
        }
        out
    }
}

/// A worker is stalled only while holding a task and idle past the threshold.
/// The one definition: the cadencer's digest lines and the load counts use it.
pub fn is_stalled(holds_task: bool, idle_s: u64, stall_after_s: u64) -> bool {
    holds_task && idle_s >= stall_after_s
}

/// The board's config, read from its own record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardCfg {
    /// Repos to cache, `owner/name`.
    pub repos: Vec<String>,
    /// The github connector agent (`team_github.tools`).
    pub github: String,
    /// The store that holds the queue and the board's state.
    pub store: String,
    /// Who audit findings are asked of.
    pub ask_to: String,
    /// Seconds without activity before a claim is stale.
    pub stale_secs: u64,
    /// The roles registry's id (`roles:`): the board then adds the `roles` section to its
    /// `board` reply. None when the record names none.
    pub roles: Option<String>,
}

fn cfg_of(
    kernel: &Arc<fantastic_kernel::Kernel>,
    id: &fantastic_kernel::AgentId,
) -> BoardCfg {
    let (meta, repos) = crate::puller::record_and_repos(kernel, id);
    let text = |k: &str| {
        record::RECORD
            .get(&meta, k)
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default()
    };
    BoardCfg {
        repos,
        github: text("github"),
        store: text("store"),
        ask_to: text("ask_to"),
        roles: record::RECORD
            .get(&meta, "roles")
            .and_then(|v| v.as_str().map(str::to_string)),
        stale_secs: record::RECORD
            .get(&meta, "stale_minutes")
            .and_then(|v| v.as_u64())
            .unwrap_or_default()
            * 60,
    }
}

/// Read the board from the store. A missing or unreadable file is an empty board.
pub async fn load(peers: &dyn Peers, store: &str) -> Board {
    read_text(peers, store, STATE_PATH)
        .await
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default()
}

/// Read a whole text file from the store. A content-carrying store answers
/// `content`; the file bridge answers pages of `lines` (200 a page, `eof` on
/// the last), which are read in turn and joined with newlines. `None` when the
/// store refuses or has no such file.
async fn read_text(peers: &dyn Peers, store: &str, path: &str) -> Option<String> {
    let mut text = String::new();
    let mut offset = 0u64;
    loop {
        let r = peers
            .ask(
                store,
                json!({"type": "read", "path": path, "offset": offset}),
            )
            .await;
        if r.get("error").is_some() {
            return None;
        }
        if let Some(c) = r["content"].as_str() {
            return Some(c.to_string());
        }
        let lines = r["lines"].as_array()?;
        for (i, l) in lines.iter().enumerate() {
            if offset > 0 || i > 0 {
                text.push('\n');
            }
            text.push_str(l["text"].as_str().unwrap_or(""));
        }
        offset += lines.len() as u64;
        if r["eof"].as_bool().unwrap_or(true) || lines.is_empty() {
            return Some(text);
        }
    }
}

/// Write the board to the store.
pub async fn save(peers: &dyn Peers, store: &str, board: &Board) -> Value {
    let bytes = serde_json::to_string_pretty(board).unwrap_or_default();
    peers
        .ask(
            store,
            json!({"type": "write", "path": STATE_PATH, "bytes": bytes}),
        )
        .await
}

/// Cache every open issue of every repo, and read each one's queue state.
/// Returns the errors met on the way.
pub async fn pull(peers: &dyn Peers, cfg: &BoardCfg, board: &mut Board, now: u64) -> Vec<String> {
    let mut errors = Vec::new();
    for repo in &cfg.repos {
        let listed = peers
            .ask(
                &cfg.github,
                json!({"type": "issues", "repo": repo, "state": "open"}),
            )
            .await;
        let Some(issues) = listed["issues"].as_array() else {
            errors.push(format!("{repo}: {listed}"));
            continue;
        };
        for issue in issues {
            let Some(number) = issue["number"].as_u64() else {
                continue;
            };
            let title = issue["title"].as_str().unwrap_or("");
            board.add_issue(repo, number, title, now);
            let (_, state) = task_file_names(repo, number);
            if let Some(content) = read_text(peers, &cfg.store, &state).await {
                board.apply_state(&format!("{repo}#{number}"), &content, now);
            }
        }
    }
    errors
}

/// Record a question before it is sent; returns its index in the history.
fn record_ask(board: &mut Board, to: &str, text: &str, now: u64) -> usize {
    board.asks.push(Record {
        at: now,
        author: "board".into(),
        kind: format!("ask:{to}"),
        text: text.into(),
        answer: None,
    });
    board.asks.len() - 1
}

/// Ask an agent, record the question, and record its answer when it comes back.
async fn ask_recorded(
    peers: &dyn Peers,
    board: &mut Board,
    to: &str,
    text: &str,
    now: u64,
) -> Value {
    let idx = record_ask(board, to, text, now);
    let reply = peers
        .ask(to, json!({"type": "message", "text": text}))
        .await;
    board.asks[idx].answer = Some(reply.to_string());
    reply
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

/// Who a registry task's `holder` names: an agent id, or `{subtree: id}` for a whole branch.
fn holds(holder: &Value, id: &str, ancestors: &[String]) -> bool {
    match holder {
        Value::String(h) => h == id,
        other => other
            .get("subtree")
            .and_then(Value::as_str)
            .is_some_and(|r| r == id || ancestors.iter().any(|a| a == r)),
    }
}

/// Give `node` and everything below it a role and a state; push each descendant onto `flat`.
fn annotate(
    node: &Value,
    parent: Option<&str>,
    ancestors: &mut Vec<String>,
    roles: &BTreeMap<String, String>,
    tasks: &[Value],
    flat: &mut Vec<Value>,
) -> Value {
    let id = node["id"].as_str().unwrap_or("").to_string();
    let held: Vec<&str> = tasks
        .iter()
        .filter(|t| holds(&t["holder"], &id, ancestors))
        .filter_map(|t| t["status"].as_str())
        .collect();
    let role = roles.get(&id).cloned().unwrap_or_default();
    let state = state_of(&held);
    let mut out = node.clone();
    out["role"] = json!(role);
    out["state"] = json!(state);
    if let Some(p) = parent {
        flat.push(json!({"id": id, "role": role, "state": state, "parent": p}));
    }
    ancestors.push(id.clone());
    let kids: Vec<Value> = node["children"]
        .as_array()
        .map(|c| {
            c.iter()
                .map(|k| annotate(k, Some(&id), ancestors, roles, tasks, flat))
                .collect()
        })
        .unwrap_or_default();
    ancestors.pop();
    out["children"] = json!(kids);
    out
}

/// The roles-mode section of the board from the registries' replies: `fleet` (the roles
/// registry's `fleet`), `count` (its `get_count`, passed through as it answers), `tree`
/// (the `reflect` node of the project_lead) and `tasks` (the task_registry's `list`).
/// An error reply leaves its part empty or null.
pub fn roles_view(fleet: &Value, count: &Value, tree: &Value, tasks: &Value) -> Value {
    let mut roles = BTreeMap::new();
    let mut lead: Option<String> = None;
    if let Some(m) = fleet.get("roles").and_then(Value::as_object) {
        for (role, list) in m {
            for e in list.as_array().into_iter().flatten() {
                let id = e.as_str().or_else(|| e["id"].as_str());
                if let Some(id) = id {
                    roles.insert(id.to_string(), role.clone());
                    if role == "project_lead" && lead.is_none() {
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
        annotate(tree, None, &mut Vec::new(), &roles, &rows, &mut agents)
    } else {
        Value::Null
    };
    let tasks: Vec<Value> = rows
        .iter()
        .map(|t| {
            json!({"id": t["id"], "source": t["source"], "holder": t["holder"], "status": t["status"]})
        })
        .collect();
    json!({"roles": {
        "lead": lead,
        "fleet": dispatcher::fleet_of(count),
        "tree": tree,
        "agents": agents,
        "tasks": tasks,
    }})
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

/// The four registry reads of roles mode, through the public verbs only: the roles
/// registry's `fleet` and `get_count`, the root's `reflect` of the project_lead's branch
/// and the task registry's `list`. A peer that errors leaves its part an error value
/// (or `Null` for the tree); nothing panics. Returns `(fleet, count, tree, tasks)`.
async fn roles_reads(peers: &dyn Peers, roles: &str) -> (Value, Value, Value, Value) {
    let fleet = peers.ask(roles, json!({"type": "fleet"})).await;
    let count = peers.ask(roles, json!({"type": "get_count"})).await;
    let lead = fleet["roles"]["project_lead"]
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

/// Read the roles-mode state through the public verbs only: the roles registry's `fleet`
/// and `get_count`, the root's `reflect` of the project_lead's branch and the task
/// registry's `list`. A peer that errors leaves its part empty; nothing panics.
pub async fn roles_state(peers: &dyn Peers, roles: &str) -> Value {
    let (fleet, count, tree, tasks) = roles_reads(peers, roles).await;
    roles_view(&fleet, &count, &tree, &tasks)
}

/// The newest `at` (seconds) a task shows: over its history entries and its report.
fn task_at(t: &Value) -> Option<f64> {
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
pub fn roles_rows(tree: &Value, tasks: &Value, now: u64) -> Vec<Value> {
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

/// The `load` object of roles mode:
/// the agents below the project_lead busy, idle and stalled, and the registry's tasks
/// by status, with `queued` the waiting ones, so the quiet heartbeat can fire.
pub fn roles_load(rows: &[Value], tasks: &Value, stall_after_s: u64) -> Value {
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

/// The spawned tree for the cadencer: the rows ([`roles_rows`]) and the load
/// ([`roles_load`]), read through [`roles_reads`] (public verbs only; no spawned agent
/// is called). A peer that errors gives no rows.
pub async fn roles_tree_rows(
    peers: &dyn Peers,
    roles: &str,
    now: u64,
    stall_after_s: u64,
) -> (Vec<Value>, Value) {
    let (_, _, tree, tasks) = roles_reads(peers, roles).await;
    let rows = roles_rows(&tree, &tasks, now);
    let load = roles_load(&rows, &tasks, stall_after_s);
    (rows, load)
}

async fn dispatch(peers: &dyn Peers, cfg: &BoardCfg, call: verbs::Call) -> Value {
    let now = fantastic_ai_core::helpers::now_secs() as u64;
    let mut board = load(peers, &cfg.store).await;
    let saves = matches!(
        call,
        verbs::Call::Pull(_)
            | verbs::Call::Report(_)
            | verbs::Call::Agent(_)
            | verbs::Call::Ask(_)
            | verbs::Call::Audit(_)
    );
    let out = match call {
        verbs::Call::Reflect(_) | verbs::Call::Boot(_) => Value::Null,
        verbs::Call::Pull(_) => {
            let errors = pull(peers, cfg, &mut board, now).await;
            json!({"tasks": board.tasks.len(), "errors": errors})
        }
        verbs::Call::Board(_) => {
            let mut out = board.snapshot();
            if let Some(r) = &cfg.roles {
                out["roles"] = roles_state(peers, r).await["roles"].clone();
            }
            out
        }
        verbs::Call::Task(a) => match board.tasks.get(&a.id) {
            Some(task) => json!(task),
            None => json!({"error": format!("no task `{}`", a.id)}),
        },
        verbs::Call::Report(a) => {
            match board.report(&a.task, a.status, &a.note, a.pr.as_deref(), &a.author, now) {
                Ok(()) => json!({"ok": true}),
                Err(e) => json!({"error": e}),
            }
        }
        verbs::Call::Agent(a) => {
            board.touch_agent(&a.name, &a.state, a.cost, now);
            json!({"ok": true})
        }
        verbs::Call::Ask(a) => {
            let to = a.to.as_deref().unwrap_or(cfg.ask_to.as_str());
            ask_recorded(peers, &mut board, to, &a.question, now).await
        }
        verbs::Call::Audit(_) => {
            let findings = board.audit(now, cfg.stale_secs);
            let mut texts = Vec::new();
            for finding in &findings {
                let text = finding.text();
                ask_recorded(peers, &mut board, &cfg.ask_to, &text, now).await;
                texts.push(text);
            }
            json!({"findings": texts})
        }
    };
    if saves {
        save(peers, &cfg.store, &board).await;
    }
    out
}

/// The board bundle: `pull`, `board`, `task`, `report`, `agent`, `ask`, `audit`.
#[derive(Default)]
pub struct BoardBundle;

#[async_trait::async_trait]
impl fantastic_kernel::bundle::Handler for BoardBundle {
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
        match call {
            verbs::Call::Boot(_) => {
                crate::declared_views::grant_views(kernel);
                Ok(None)
            }
            verbs::Call::Reflect(_) => Ok(Some(json!({"id": agent_id.as_str(), "sentence": MAN}))),
            call => {
                let cfg = cfg_of(kernel, agent_id);
                Ok(Some(dispatch(&KernelPeers(kernel.clone()), &cfg, call).await))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_boot::{boot_at, Seed};
    use std::sync::Mutex;

    fn task(id: &str, status: Status, claimers: &[&str], pr: Option<&str>, updated: u64) -> Task {
        Task {
            id: id.into(),
            title: String::new(),
            status,
            claimers: claimers.iter().map(|c| c.to_string()).collect(),
            pr: pr.map(str::to_string),
            note: None,
            updated,
        }
    }

    /// Mock github connector and store: issues 25 (labelled) and 26 (not),
    /// and a files map for the store.
    struct Fake {
        files: Mutex<BTreeMap<String, String>>,
    }

    #[async_trait::async_trait]
    impl Peers for Fake {
        async fn ask(&self, to: &str, p: Value) -> Value {
            match (to, p["type"].as_str().unwrap_or("")) {
                ("gh", "issues") => json!({"issues": [
                    {"number": 25, "title": "labelled", "labels": ["team-ready"]},
                    {"number": 26, "title": "unlabelled", "labels": []}
                ]}),
                ("store", "read") => match self
                    .files
                    .lock()
                    .unwrap()
                    .get(p["path"].as_str().unwrap_or(""))
                {
                    Some(c) => json!({"content": c}),
                    None => json!({"error": "not found"}),
                },
                ("store", "write") => {
                    let path = p["path"].as_str().unwrap_or("").to_string();
                    let bytes = p["bytes"].as_str().unwrap_or("").to_string();
                    self.files.lock().unwrap().insert(path, bytes);
                    json!({"ok": true})
                }
                _ => json!({"reply": "noted"}),
            }
        }
    }

    fn cfg() -> BoardCfg {
        BoardCfg {
            repos: vec!["aisixteen/fantastic_team".into()],
            github: "gh".into(),
            store: "store".into(),
            ask_to: "user".into(),
            stale_secs: 1800,
            roles: None,
        }
    }

    #[tokio::test]
    async fn pull_caches_all_open_issues_from_mock_github() {
        let fake = Fake {
            files: Mutex::new(BTreeMap::new()),
        };
        let mut board = Board::default();
        let errors = pull(&fake, &cfg(), &mut board, 100).await;
        assert!(errors.is_empty());
        assert_eq!(board.tasks.len(), 2);
        assert_eq!(
            board.tasks["aisixteen/fantastic_team#25"].status,
            Status::Pending
        );
        assert_eq!(
            board.tasks["aisixteen/fantastic_team#26"].status,
            Status::Pending
        );
    }

    /// A file bridge store: a `read` answers pages of `lines` (two a page, `eof` on
    /// the last), never `content`; a `write` keeps the bytes.
    struct Bridge {
        files: Mutex<BTreeMap<String, String>>,
    }

    #[async_trait::async_trait]
    impl Peers for Bridge {
        async fn ask(&self, to: &str, p: Value) -> Value {
            match (to, p["type"].as_str().unwrap_or("")) {
                ("gh", "issues") => json!({"issues": [{"number": 1, "title": "t"}]}),
                ("store", "read") => {
                    let files = self.files.lock().unwrap();
                    let Some(text) = files.get(p["path"].as_str().unwrap_or("")) else {
                        return json!({"error": "not found"});
                    };
                    let all: Vec<&str> = text.lines().collect();
                    let from = (p["offset"].as_u64().unwrap_or(0) as usize).min(all.len());
                    let page: Vec<Value> = all[from..]
                        .iter()
                        .take(2)
                        .enumerate()
                        .map(|(i, l)| json!({"n": from + i + 1, "text": l}))
                        .collect();
                    let eof = from + page.len() >= all.len();
                    json!({"lines": page, "eof": eof})
                }
                ("store", "write") => {
                    let path = p["path"].as_str().unwrap_or("").to_string();
                    let bytes = p["bytes"].as_str().unwrap_or("").to_string();
                    self.files.lock().unwrap().insert(path, bytes);
                    json!({"ok": true})
                }
                _ => json!({"error": "unknown"}),
            }
        }
    }

    #[tokio::test]
    async fn pull_reads_queue_state_from_a_file_bridge_store() {
        let mut files = BTreeMap::new();
        files.insert(
            "queue/aisixteen-fantastic_team-1.state".to_string(),
            "in_work lead\n".to_string(),
        );
        let bridge = Bridge {
            files: Mutex::new(files),
        };
        let mut board = Board::default();
        let errors = pull(&bridge, &cfg(), &mut board, 100).await;
        assert!(errors.is_empty(), "{errors:?}");
        let t = &board.tasks["aisixteen/fantastic_team#1"];
        assert_eq!(t.status, Status::InWork);
        assert_eq!(t.claimers, vec!["lead".to_string()]);
    }

    #[tokio::test]
    async fn save_then_load_round_trips_through_a_paged_file_bridge() {
        let bridge = Bridge {
            files: Mutex::new(BTreeMap::new()),
        };
        let mut board = Board::default();
        for n in 1..=5 {
            board.add_issue("o/r", n, "t", 1);
        }
        board.apply_state("o/r#3", "done o/r#9", 2);
        save(&bridge, "store", &board).await;
        let back = load(&bridge, "store").await;
        assert_eq!(back.tasks.len(), 5, "all pages are read");
        assert_eq!(back.tasks["o/r#3"].status, Status::Done);
        assert_eq!(load(&bridge, "nowhere").await.tasks.len(), 0);
    }

    #[test]
    fn claim_file_moves_task_to_in_work() {
        let mut board = Board::default();
        board.add_issue("o/r", 1, "t", 0);
        board.apply_state("o/r#1", "in_work alice\n", 5);
        let t = &board.tasks["o/r#1"];
        assert_eq!(t.status, Status::InWork);
        assert_eq!(t.claimers, vec!["alice".to_string()]);
    }

    #[test]
    fn report_done_with_pr_moves_task_to_done() {
        let mut board = Board::default();
        board.add_issue("o/r", 2, "t", 0);
        board
            .report(
                "o/r#2",
                Status::Done,
                "shipped",
                Some("https://pr/9"),
                "alice",
                7,
            )
            .unwrap();
        let t = &board.tasks["o/r#2"];
        assert_eq!(t.status, Status::Done);
        assert_eq!(t.pr.as_deref(), Some("https://pr/9"));
        assert_eq!(board.prs["https://pr/9"], "o/r#2");
    }

    #[test]
    fn audit_flags_stale_claim() {
        let mut board = Board::default();
        board.tasks.insert(
            "o/r#1".into(),
            task("o/r#1", Status::InWork, &["a"], None, 0),
        );
        assert_eq!(
            board.audit(10_000, 60),
            vec![Finding::StaleClaim {
                task: "o/r#1".into()
            }]
        );
    }

    #[test]
    fn audit_flags_unclaimed_work() {
        let mut board = Board::default();
        board
            .tasks
            .insert("o/r#1".into(), task("o/r#1", Status::InWork, &[], None, 0));
        assert_eq!(
            board.audit(0, 60),
            vec![Finding::UnclaimedWork {
                task: "o/r#1".into()
            }]
        );
    }

    #[test]
    fn audit_flags_double_claim() {
        let mut board = Board::default();
        board.tasks.insert(
            "o/r#1".into(),
            task("o/r#1", Status::InWork, &["a", "b"], None, 0),
        );
        assert_eq!(
            board.audit(0, 60),
            vec![Finding::DoubleClaim {
                task: "o/r#1".into()
            }]
        );
    }

    #[test]
    fn audit_flags_done_without_pr() {
        let mut board = Board::default();
        board
            .tasks
            .insert("o/r#1".into(), task("o/r#1", Status::Done, &["a"], None, 0));
        assert_eq!(
            board.audit(0, 60),
            vec![Finding::DoneWithoutPr {
                task: "o/r#1".into()
            }]
        );
    }

    #[test]
    fn audit_flags_pr_without_task() {
        let mut board = Board::default();
        board.prs.insert("https://pr/3".into(), "ghost#3".into());
        assert_eq!(
            board.audit(0, 60),
            vec![Finding::PrWithoutTask {
                pr: "https://pr/3".into()
            }]
        );
    }

    #[test]
    fn audit_flags_repeated_failure() {
        let mut board = Board::default();
        board.failures.insert("alice".into(), FAIL_LIMIT);
        assert_eq!(
            board.audit(0, 60),
            vec![Finding::RepeatedFailure {
                agent: "alice".into()
            }]
        );
    }

    #[test]
    fn audit_never_assigns() {
        let mut board = Board::default();
        board
            .tasks
            .insert("o/r#1".into(), task("o/r#1", Status::InWork, &[], None, 0));
        let before = board.tasks.clone();
        let findings = board.audit(0, 60);
        assert_eq!(findings.len(), 1);
        assert_eq!(board.tasks, before);
    }

    #[tokio::test]
    async fn ask_is_recorded_with_answer() {
        let fake = Fake {
            files: Mutex::new(BTreeMap::new()),
        };
        let mut board = Board::default();
        ask_recorded(&fake, &mut board, "user", "ship it?", 3).await;
        assert_eq!(board.asks.len(), 1);
        assert_eq!(board.asks[0].text, "ship it?");
        assert!(board.asks[0].answer.as_deref().unwrap().contains("noted"));
    }

    #[test]
    fn board_reply_shape_is_stable() {
        let v = Board::default().snapshot();
        for k in ["pending", "in_work", "done", "failed"] {
            assert!(v["tasks"][k].is_array(), "missing tasks.{k}");
        }
        assert!(v["agents"].is_object());
    }

    #[test]
    fn register_team_registers_board() {
        let mut reg = fantastic_kernel::BundleRegistry::new();
        crate::register_team(&mut reg);
        assert!(reg.get(HANDLER_MODULE).is_some());
    }

    // ---- roles mode (#193) ----

    /// A scripted roles registry, kernel reflect and task registry; records every call.
    struct RolesPeers {
        fleet: Value,
        count: Value,
        reflect: Value,
        tasks: Value,
        calls: Mutex<Vec<(String, Value)>>,
    }

    #[async_trait::async_trait]
    impl Peers for RolesPeers {
        async fn ask(&self, to: &str, p: Value) -> Value {
            self.calls.lock().unwrap().push((to.to_string(), p.clone()));
            match (to, p["type"].as_str().unwrap_or("")) {
                ("roles", "fleet") => self.fleet.clone(),
                ("roles", "get_count") => self.count.clone(),
                ("kernel", "reflect") => self.reflect.clone(),
                ("tasks", "list") => self.tasks.clone(),
                ("store", "read") => json!({"error": "not found"}),
                _ => json!({"error": "unexpected"}),
            }
        }
    }

    fn roles_peers() -> RolesPeers {
        RolesPeers {
            fleet: json!({"roles": {"project_lead": [{"id": "pl"}], "worker": [{"id": "w1"}, {"id": "w2"}]}}),
            count: json!({"running": 3, "max": 5}),
            reflect: json!({"tree": [{"id": "pl", "children": [{"id": "w1", "children": []}, {"id": "w2", "children": []}]}]}),
            tasks: json!({"tasks": [
                {"id": "t1", "source": "issue 7", "holder": "w1", "status": "running", "history": []}
            ]}),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn fleet_pl() -> Value {
        json!({"roles": {"project_lead": [{"id": "pl"}], "worker": [{"id": "w1"}, {"id": "w2"}]}})
    }

    fn tree_pl() -> Value {
        json!({"id": "pl", "children": [
            {"id": "w1", "children": []}, {"id": "w2", "children": []}
        ]})
    }

    fn state_of_agent(v: &Value, id: &str) -> Value {
        v["roles"]["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == id)
            .unwrap_or_else(|| panic!("no agent {id}"))["state"]
            .clone()
    }

    #[test]
    fn roles_view_lists_the_tree_below_the_project_lead_with_roles() {
        let v = roles_view(&fleet_pl(), &json!({}), &tree_pl(), &json!({"tasks": []}));
        assert_eq!(v["roles"]["lead"], "pl");
        assert_eq!(v["roles"]["tree"]["id"], "pl");
        assert_eq!(v["roles"]["tree"]["role"], "project_lead");
        assert_eq!(v["roles"]["tree"]["children"][0]["role"], "worker");
        let agents = v["roles"]["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 2, "the lead is the root, not listed again");
        for id in ["w1", "w2"] {
            let a = agents.iter().find(|a| a["id"] == id).unwrap();
            assert_eq!(a["role"], "worker");
            assert_eq!(a["parent"], "pl");
        }
    }

    #[test]
    fn roles_view_agent_state_comes_from_its_tasks() {
        let fleet = json!({"roles": {"project_lead": [{"id": "pl"}], "worker": [
            {"id": "w1"}, {"id": "w2"}, {"id": "w3"}, {"id": "w4"}]}});
        let tree = json!({"id": "pl", "children": [
            {"id": "w1", "children": []}, {"id": "w2", "children": []},
            {"id": "w3", "children": []}, {"id": "w4", "children": []}]});
        let tasks = json!({"tasks": [
            {"id": "a", "source": "s", "holder": "w1", "status": "running"},
            {"id": "b", "source": "s", "holder": "w2", "status": "paused"},
            {"id": "c", "source": "s", "holder": "w3", "status": "done"}
        ]});
        let v = roles_view(&fleet, &json!({}), &tree, &tasks);
        assert_eq!(state_of_agent(&v, "w1"), "active");
        assert_eq!(state_of_agent(&v, "w2"), "paused");
        assert_eq!(state_of_agent(&v, "w3"), "done");
        assert_eq!(state_of_agent(&v, "w4"), "active");
    }

    #[test]
    fn roles_view_subtree_holder_covers_descendants() {
        let tree = json!({"id": "pl", "children": [
            {"id": "sub", "children": [{"id": "kid", "children": []}]},
            {"id": "other", "children": []}]});
        let tasks = json!({"tasks": [
            {"id": "a", "source": "s", "holder": {"subtree": "sub"}, "status": "paused"}
        ]});
        let v = roles_view(&json!({}), &json!({}), &tree, &tasks);
        assert_eq!(state_of_agent(&v, "sub"), "paused");
        assert_eq!(state_of_agent(&v, "kid"), "paused");
        assert_eq!(state_of_agent(&v, "other"), "active");
    }

    #[test]
    fn roles_view_lists_tasks_with_source_holder_status() {
        let tasks = json!({"tasks": [
            {"id": "t1", "source": "issue 7", "holder": "w1", "status": "running", "history": [1]},
            {"id": "t2", "source": "chat", "holder": {"subtree": "sub"}, "status": "waiting", "history": []}
        ]});
        let v = roles_view(&fleet_pl(), &json!({}), &tree_pl(), &tasks);
        assert_eq!(
            v["roles"]["tasks"],
            json!([
                {"id": "t1", "source": "issue 7", "holder": "w1", "status": "running"},
                {"id": "t2", "source": "chat", "holder": {"subtree": "sub"}, "status": "waiting"}
            ])
        );
    }

    #[test]
    fn roles_view_fleet_is_the_registrys_count_as_is() {
        let v = roles_view(
            &json!({}),
            &json!({"running": 3, "max": 5}),
            &Value::Null,
            &json!({}),
        );
        assert_eq!(v["roles"]["fleet"], json!({"running": 3, "max": 5}));
        let v = roles_view(
            &json!({}),
            &json!({"error": "no"}),
            &Value::Null,
            &json!({}),
        );
        assert!(v["roles"]["fleet"].is_null());
    }

    #[tokio::test]
    async fn roles_state_reads_through_public_verbs() {
        let peers = roles_peers();
        let v = roles_state(&peers, "roles").await;
        let calls: Vec<(String, String)> = peers
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|(to, p)| (to.clone(), p["type"].as_str().unwrap_or("").to_string()))
            .collect();
        let want = [
            ("roles", "fleet"),
            ("roles", "get_count"),
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
        assert_eq!(v["roles"]["tree"]["id"], "pl");
        assert_eq!(v["roles"]["fleet"], json!({"running": 3, "max": 5}));
        assert_eq!(v["roles"]["tasks"][0]["id"], "t1");
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
    fn roles_rows_has_the_feed_shape() {
        let tree = json!({"id": "pl", "children": [
            {"id": "sub", "children": [{"id": "w1", "children": []}]},
            {"id": "w2", "children": []}]});
        let tasks = json!({"tasks": [
            {"id": "t1", "source": "s", "holder": "w1", "status": "running",
             "history": history(900.0), "report": {"from": "w1", "at": 940.0}},
            {"id": "t2", "source": "s", "holder": "w2", "status": "waiting", "history": history(990.0)}
        ]});
        let rows = roles_rows(&tree, &tasks, 1000);
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
    fn roles_rows_subtree_holder_marks_descendants_busy() {
        let tree = json!({"id": "pl", "children": [
            {"id": "sub", "children": [{"id": "kid", "children": []}]},
            {"id": "other", "children": []}]});
        let tasks = json!({"tasks": [
            {"id": "a", "source": "s", "holder": {"subtree": "sub"}, "status": "running", "history": history(500.0)}
        ]});
        let rows = roles_rows(&tree, &tasks, 600);
        assert_eq!(row_of(&rows, "sub")["state"], "busy");
        assert_eq!(row_of(&rows, "kid")["state"], "busy");
        assert_eq!(row_of(&rows, "kid")["task"], "a");
        assert_eq!(row_of(&rows, "other")["state"], "idle");
        assert_eq!(row_of(&rows, "pl")["state"], "idle");
    }

    #[test]
    fn roles_rows_idle_agent_without_task_is_idle_not_stalled() {
        let tree = json!({"id": "pl", "children": [
            {"id": "w1", "children": []}, {"id": "w2", "children": []}]});
        let tasks = json!({"tasks": [
            {"id": "a", "source": "s", "holder": "w2", "status": "done", "history": history(1.0)}
        ]});
        let rows = roles_rows(&tree, &tasks, 100_000);
        let w1 = row_of(&rows, "w1");
        assert_eq!(
            (w1["state"].as_str(), w1["task"].as_str()),
            (Some("idle"), Some(""))
        );
        assert_eq!(w1["idle_s"], 0);
        assert_eq!(row_of(&rows, "w2")["state"], "done");
        assert_eq!(row_of(&rows, "w2")["task"], "");
        let load = roles_load(&rows, &tasks, 300);
        assert_eq!(load["workers"]["stalled"], 0);
        assert_eq!(load["workers"]["idle"], 1);
    }

    #[test]
    fn roles_load_counts_queued_and_stalled() {
        let tree = json!({"id": "pl", "children": [{"id": "w1", "children": []}]});
        let tasks = json!({"tasks": [
            {"id": "a", "source": "s", "holder": "w1", "status": "running", "history": history(0.0)},
            {"id": "b", "source": "s", "holder": null, "status": "waiting", "history": []},
            {"id": "c", "source": "s", "holder": null, "status": "waiting", "history": []}
        ]});
        let rows = roles_rows(&tree, &tasks, 1000);
        let load = roles_load(&rows, &tasks, 300);
        assert_eq!(load["tasks"]["queued"], 2);
        assert_eq!(load["tasks"]["held"], 1);
        assert_eq!(load["workers"]["busy"], 1);
        assert_eq!(load["workers"]["stalled"], 1);
    }

    #[tokio::test]
    async fn roles_tree_rows_reads_the_public_verbs_only() {
        let peers = roles_peers();
        let (rows, load) = roles_tree_rows(&peers, "roles", 1000, 300).await;
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
                "roles:fleet",
                "roles:get_count",
                "kernel:reflect",
                "tasks:list"
            ]
        );
    }

    #[tokio::test]
    async fn roles_tree_rows_survives_peer_errors() {
        let e = json!({"error": "down"});
        let peers = RolesPeers {
            fleet: e.clone(),
            count: e.clone(),
            reflect: e.clone(),
            tasks: e,
            calls: Mutex::new(Vec::new()),
        };
        let (rows, load) = roles_tree_rows(&peers, "roles", 1000, 300).await;
        assert!(rows.is_empty());
        assert_eq!(load["tasks"]["queued"], 0);
    }

    #[tokio::test]
    async fn roles_state_survives_errors() {
        let e = json!({"error": "down"});
        let peers = RolesPeers {
            fleet: e.clone(),
            count: e.clone(),
            reflect: e.clone(),
            tasks: e,
            calls: Mutex::new(Vec::new()),
        };
        let v = roles_state(&peers, "roles").await;
        assert!(v["roles"]["tree"].is_null());
        assert!(v["roles"]["lead"].is_null());
        assert_eq!(v["roles"]["agents"], json!([]));
        assert_eq!(v["roles"]["tasks"], json!([]));
    }

    fn fake() -> Fake {
        Fake {
            files: Mutex::new(BTreeMap::new()),
        }
    }

    #[tokio::test]
    async fn board_verb_without_roles_keeps_the_old_snapshot() {
        let peers = fake();
        let got = dispatch(&peers, &cfg(), verbs::Call::Board(verbs::Board {})).await;
        assert_eq!(got, Board::default().snapshot());
        assert!(got.get("roles").is_none());
    }

    #[tokio::test]
    async fn board_verb_with_roles_adds_the_roles_section_and_keeps_the_old_keys() {
        let peers = roles_peers();
        let mut c = cfg();
        c.roles = Some("roles".into());
        let got = dispatch(&peers, &c, verbs::Call::Board(verbs::Board {})).await;
        assert!(got.get("tasks").is_some());
        assert!(got.get("agents").is_some());
        assert_eq!(got["roles"]["tasks"][0]["id"], "t1");
        assert_eq!(got["roles"]["agents"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn cfg_of_reads_roles_from_the_board_record() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = boot_at(dir.path(), Seed::Stock, &[], None).await;
        let mut answers = Vec::new();
        for (id, roles) in [("board_r", Some("roles")), ("board_n", None)] {
            let mut rec = json!({
                "type": "create_agent", "id": id, "parent_id": "kernel_state",
                "handler_module": HANDLER_MODULE, "github": "gh", "store": "store"
            });
            if let Some(r) = roles {
                rec["roles"] = json!(r);
            }
            let made = kernel
                .send(vec![fantastic_kernel::Item::new("kernel", rec)])
                .await;
            assert!(
                matches!(made[0].status, fantastic_kernel::Status::Completed),
                "{:?}",
                made[0].result
            );
            let got = kernel
                .send(vec![fantastic_kernel::Item::new(
                    id,
                    json!({"type": "board"}),
                )])
                .await;
            answers.push(got[0].result.clone());
        }
        assert!(answers[0].get("roles").is_some(), "{}", answers[0]);
        assert!(answers[1].get("roles").is_none(), "{}", answers[1]);
        kernel.stop().await;
    }
}
