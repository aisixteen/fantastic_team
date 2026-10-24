//! Board: the shift's shared picture of the work. It caches every open issue
//! of the configured repos (through `team_github.tools`) with its queue state
//! (`queue/*.state`), knows the agents, keeps its own state file in the store,
//! and audits the picture. It NEVER assigns: no verb sets an assignee or writes
//! a claim for someone else. Audit findings only become asks.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::puller::{task_file_names, KernelPeers, Peers};

/// The bundle's handler module name.
pub const HANDLER_MODULE: &str = "team_board.tools";
/// Failures by one agent before the audit flags it.
pub const FAIL_LIMIT: u32 = 3;
/// Where the board keeps its state inside the store.
const STATE_PATH: &str = "board/state.json";

/// Where a task stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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

/// A configured group: its AI driver (empty when it has none) and its members.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    /// The agent that receives the group's digest. Empty for no driver.
    #[serde(default)]
    pub driver: String,
    /// Agent ids in the group.
    #[serde(default)]
    pub members: Vec<String>,
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

/// What the kernel says about one agent right now.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Live {
    /// Whether it is generating a reply.
    pub generating: bool,
    /// Its cache hit rate, 0..1.
    pub cache_hit: f64,
    /// The text of the inquiry its driver opened and it has not answered, if any.
    pub inquiry: Option<String>,
    /// The time of its newest conversation row, unix seconds, when known.
    pub newest_row: Option<u64>,
    /// Its cumulative token count (prompt plus output) from its `reflect` usage.
    pub tokens: u64,
}

/// Seconds without activity after which a worker holding a task is stalled:
/// the cadencer's default `stall_after`.
pub const DEFAULT_STALL_AFTER_S: u64 = 300;
/// The window the throughput figure counts over, seconds.
pub const LOAD_WINDOW_S: u64 = 3600;

/// A worker is stalled only while holding a task and idle past the threshold.
/// The one definition: the cadencer's digest lines and the load counts use it.
pub fn is_stalled(holds_task: bool, idle_s: u64, stall_after_s: u64) -> bool {
    holds_task && idle_s >= stall_after_s
}

/// One row per agent the board knows, in the shape `cadence_feed::tree_from_board` reads.
/// Rows for the board: one row per configured group (`parent` null, `driver` set),
/// then one row per agent. A member of a group has `parent` = the group, and gets a
/// row with live state even if it never registered.
pub fn rows_from(
    board: &Board,
    groups: &BTreeMap<String, Group>,
    live: &BTreeMap<String, Live>,
    now: u64,
) -> Vec<Value> {
    let mut b = board.clone();
    for g in groups.values() {
        for m in &g.members {
            b.agents.entry(m.clone()).or_insert_with(|| AgentInfo {
                state: String::new(),
                last_seen: now,
                cost: 0.0,
            });
        }
    }
    let mut rows = agent_rows(&b, live, now);
    for row in rows.iter_mut() {
        let id = row["id"].as_str().unwrap_or("").to_string();
        if let Some((gname, _)) = groups.iter().find(|(_, g)| g.members.contains(&id)) {
            row["parent"] = json!(gname);
        }
    }
    let mut out = Vec::new();
    for (gname, g) in groups {
        out.push(json!({
            "id": gname,
            "parent": null,
            "has_ai": !g.driver.is_empty(),
            "driver": g.driver,
            "state": "group",
            "task": "",
            "idle_s": 0,
            "time_on_task_s": 0,
            "cost": 0.0,
            "cache_hit": 0.0,
            "new_results": 0,
        }));
    }
    out.extend(rows);
    out
}

fn agent_rows(board: &Board, live: &BTreeMap<String, Live>, now: u64) -> Vec<Value> {
    board
        .agents
        .iter()
        .map(|(name, info)| {
            let claim = board
                .tasks
                .values()
                .find(|t| t.status == Status::InWork && t.claimers.contains(name));
            let lv = live.get(name).cloned().unwrap_or_default();
            let busy = lv.generating || claim.is_some() || lv.inquiry.is_some();
            let task = claim
                .map(|t| t.id.clone())
                .or_else(|| lv.inquiry.as_deref().map(first_line))
                .unwrap_or_default();
            let idle_s = match lv.newest_row {
                Some(t) => now
                    .saturating_sub(info.last_seen)
                    .min(now.saturating_sub(t)),
                None => now.saturating_sub(info.last_seen),
            };
            let done = board
                .tasks
                .values()
                .any(|t| t.status == Status::Done && t.claimers.contains(name));
            let failed = board
                .tasks
                .values()
                .any(|t| t.status == Status::Failed && t.claimers.contains(name));
            let state = if busy {
                "busy"
            } else if done {
                "done"
            } else if failed {
                "failed"
            } else {
                "idle"
            };
            json!({
                "id": name,
                "parent": null,
                "has_ai": true,
                "state": state,
                "task": task,
                "idle_s": idle_s,
                "time_on_task_s": claim.map(|t| now.saturating_sub(t.updated)).unwrap_or(0),
                "cost": info.cost,
                "cache_hit": lv.cache_hit,
                "new_results": 0,
            })
        })
        .collect()
}

/// How an inquiry's first row begins (ai-core's `inquiry_head`).
const INQUIRY_HEAD: &str = "[inquiry from ";

/// Worker counts: busy, idle and stalled. Each worker is counted once.
#[derive(Default)]
struct Workers {
    busy: u64,
    idle: u64,
    stalled: u64,
}

impl Workers {
    fn of<'a>(rows: impl Iterator<Item = &'a Value>, stall_after_s: u64) -> Self {
        let mut w = Workers::default();
        for r in rows.filter(|r| r["state"].as_str() != Some("group")) {
            let holds = !r["task"].as_str().unwrap_or("").is_empty();
            let idle_s = r["idle_s"].as_u64().unwrap_or(0);
            if is_stalled(holds, idle_s, stall_after_s) {
                w.stalled += 1;
            } else if r["state"].as_str() == Some("busy") {
                w.busy += 1;
            } else {
                w.idle += 1;
            }
        }
        w
    }

    fn json(&self) -> Value {
        json!({"busy": self.busy, "idle": self.idle, "stalled": self.stalled})
    }
}

/// The load the board and the cadencer already hold, as plain integer counts.
/// Pure: reads the board, the groups, the live data and `now`. It has no mode,
/// no total and no label; the owning agent decides what to show.
///
/// Shape: `workers` and `groups.<name>` are {busy, idle, stalled}; `tasks` is
/// {held, in_review, done, blocked, queued}; `done_last_hour` counts Done tasks
/// updated inside `LOAD_WINDOW_S`; `tokens_total` is the cumulative token count
/// of the group drivers only (the cadencer turns its change into tokens per hour;
/// workers' tokens are not counted). `groups.<g>.driver` is the group's driver id,
/// empty when it has none.
/// `waiting_ci` is absent on purpose: it needs a GitHub `pr_status` poll, which
/// the load view does not make.
pub fn load_numbers(
    board: &Board,
    groups: &BTreeMap<String, Group>,
    live: &BTreeMap<String, Live>,
    now: u64,
    stall_after_s: u64,
) -> Value {
    let rows = rows_from(board, groups, live, now);
    let mut group_load = serde_json::Map::new();
    for gname in groups.keys() {
        let members = rows
            .iter()
            .filter(|r| r["parent"].as_str() == Some(gname.as_str()));
        let mut g = Workers::of(members, stall_after_s).json();
        g["driver"] = json!(groups[gname].driver);
        group_load.insert(gname.clone(), g);
    }
    let drivers: BTreeSet<&str> = groups
        .values()
        .map(|g| g.driver.as_str())
        .filter(|d| !d.is_empty())
        .collect();
    let driver_tokens: u64 = live
        .iter()
        .filter(|(name, _)| drivers.contains(name.as_str()))
        .map(|(_, l)| l.tokens)
        .sum();
    let tasks = board.tasks.values();
    let count = |f: &dyn Fn(&Task) -> bool| tasks.clone().filter(|t| f(t)).count() as u64;
    let done_last_hour =
        count(&|t| t.status == Status::Done && now.saturating_sub(t.updated) < LOAD_WINDOW_S);
    json!({
        "workers": Workers::of(rows.iter(), stall_after_s).json(),
        "groups": group_load,
        "tasks": {
            "held": count(&|t| t.status == Status::InWork),
            "in_review": count(&|t| t.status == Status::Done && t.pr.is_some()),
            "done": count(&|t| t.status == Status::Done),
            "blocked": count(&|t| t.status == Status::Failed),
            "queued": count(&|t| t.status == Status::Pending),
        },
        "done_last_hour": done_last_hour,
        "tokens_total": driver_tokens,
    })
}

fn content_of(row: &Value) -> &str {
    row["content"].as_str().unwrap_or("")
}

/// The asker an inquiry row names in its header, `None` when the row is not an inquiry.
fn asker_of(content: &str) -> Option<&str> {
    let rest = content.strip_prefix(INQUIRY_HEAD)?;
    let end = rest.find(" · ").or_else(|| rest.find(']'))?;
    Some(rest[..end].trim())
}

/// The first non-empty line of a text.
fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

/// The open inquiry in an agent's conversation rows (the `rows` verb), as its text
/// after the header line. The newest row that starts with `[inquiry from <driver> `
/// is open unless a later row of the agent holds an `inquiry_answer` call. With no
/// driver known, an inquiry from anyone counts.
pub fn open_inquiry(rows: &[Value], driver: &str) -> Option<String> {
    let at = rows
        .iter()
        .rposition(|r| asker_of(content_of(r)).is_some_and(|a| driver.is_empty() || a == driver))?;
    let answered = rows[at + 1..].iter().any(|r| {
        let c = content_of(r);
        !c.starts_with(INQUIRY_HEAD) && c.contains("name=\"inquiry_answer\"")
    });
    if answered {
        return None;
    }
    let body = content_of(&rows[at])
        .split_once('\n')
        .map(|(_, b)| b.trim().to_string())
        .unwrap_or_default();
    Some(body)
}

/// The time of the newest row, unix seconds.
pub fn newest_row(rows: &[Value]) -> Option<u64> {
    rows.iter()
        .filter_map(|r| r["ts"].as_f64())
        .map(|t| t as u64)
        .max()
}

/// Ask one agent for its status, usage and conversation rows through the kernel.
/// `driver` is the agent that drives it, empty when it has none.
async fn live_of(peers: &dyn Peers, name: &str, driver: &str) -> Live {
    let status = peers.ask(name, json!({"type": "status"})).await;
    let reflect = peers.ask(name, json!({"type": "reflect"})).await;
    let history = peers.ask(name, json!({"type": "rows"})).await;
    let rows: Vec<Value> = history["rows"].as_array().cloned().unwrap_or_default();
    let usage = &reflect["usage"];
    Live {
        generating: status["status"].as_str() == Some("generating"),
        cache_hit: reflect["cache_hit"].as_f64().unwrap_or(0.0),
        inquiry: open_inquiry(&rows, driver),
        newest_row: newest_row(&rows),
        tokens: usage["prompt_tokens"].as_u64().unwrap_or(0)
            + usage["output_tokens"].as_u64().unwrap_or(0),
    }
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
    /// Groups from the board record: name → driver and members.
    pub groups: BTreeMap<String, Group>,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn cfg_of(
    kernel: &Arc<fantastic_kernel::Kernel>,
    id: &fantastic_kernel::AgentId,
) -> Result<BoardCfg, String> {
    let meta = kernel
        .agents
        .get(id)
        .map(|e| e.meta.read().expect("meta").clone())
        .unwrap_or_default();
    let text = |k: &str| {
        meta.get(k)
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|v| !v.trim().is_empty())
    };
    let repos = meta
        .get("repos")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Ok(BoardCfg {
        repos,
        github: text("github").ok_or_else(|| "team_board: record needs `github`".to_string())?,
        store: text("store").ok_or_else(|| "team_board: record needs `store`".to_string())?,
        ask_to: text("ask_to").unwrap_or_else(|| "user".into()),
        groups: meta
            .get("team_groups")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
        stale_secs: meta
            .get("stale_minutes")
            .and_then(|v| v.as_u64())
            .unwrap_or(30)
            * 60,
    })
}

/// Read the board from the store. A missing or unreadable file is an empty board.
pub async fn load(peers: &dyn Peers, store: &str) -> Board {
    let r = peers
        .ask(store, json!({"type": "read", "path": STATE_PATH}))
        .await;
    r["content"]
        .as_str()
        .and_then(|c| serde_json::from_str(c).ok())
        .unwrap_or_default()
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
            let read = peers
                .ask(&cfg.store, json!({"type": "read", "path": state}))
                .await;
            if let Some(content) = read["content"].as_str() {
                board.apply_state(&format!("{repo}#{number}"), content, now);
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

async fn dispatch(peers: &dyn Peers, cfg: &BoardCfg, t: &str, p: &Value) -> Value {
    let now = now_secs();
    let mut board = load(peers, &cfg.store).await;
    let out = match t {
        "pull" => {
            let errors = pull(peers, cfg, &mut board, now).await;
            json!({"tasks": board.tasks.len(), "errors": errors})
        }
        "board" => board.snapshot(),
        "rows" => {
            let mut live = BTreeMap::new();
            let mut names: BTreeSet<String> = board.agents.keys().cloned().collect();
            for g in cfg.groups.values() {
                names.extend(g.members.iter().cloned());
                // A driver is read even when it is in no group's members.
                if !g.driver.is_empty() {
                    names.insert(g.driver.clone());
                }
            }
            for name in &names {
                let driver = cfg
                    .groups
                    .values()
                    .find(|g| g.members.contains(name))
                    .map(|g| g.driver.as_str())
                    .unwrap_or("");
                live.insert(name.clone(), live_of(peers, name, driver).await);
            }
            let stall_after_s = p["stall_after_s"].as_u64().unwrap_or(DEFAULT_STALL_AFTER_S);
            json!({
                "rows": rows_from(&board, &cfg.groups, &live, now),
                "load": load_numbers(&board, &cfg.groups, &live, now, stall_after_s),
            })
        }
        "task" => {
            let id = p["id"].as_str().unwrap_or("");
            match board.tasks.get(id) {
                Some(task) => json!(task),
                None => json!({"error": format!("no task `{id}`")}),
            }
        }
        "report" => {
            let id = p["task"].as_str().unwrap_or("");
            match p["status"].as_str().and_then(Status::parse) {
                None => json!({"error": "report needs status pending|in_work|done|failed"}),
                Some(status) => match board.report(
                    id,
                    status,
                    p["note"].as_str().unwrap_or(""),
                    p["pr"].as_str(),
                    p["author"].as_str().unwrap_or("unknown"),
                    now,
                ) {
                    Ok(()) => json!({"ok": true}),
                    Err(e) => json!({"error": e}),
                },
            }
        }
        "agent" => {
            let name = p["name"].as_str().unwrap_or("");
            board.touch_agent(
                name,
                p["state"].as_str().unwrap_or(""),
                p["cost"].as_f64().unwrap_or(0.0),
                now,
            );
            json!({"ok": true})
        }
        "ask" => {
            let to = p["to"].as_str().unwrap_or(cfg.ask_to.as_str());
            let question = p["question"].as_str().unwrap_or("");
            ask_recorded(peers, &mut board, to, question, now).await
        }
        "audit" => {
            let findings = board.audit(now, cfg.stale_secs);
            let mut texts = Vec::new();
            for finding in &findings {
                let text = finding.text();
                ask_recorded(peers, &mut board, &cfg.ask_to, &text, now).await;
                texts.push(text);
            }
            json!({"findings": texts})
        }
        other => json!({"error": format!("team_board: unknown type '{other}'")}),
    };
    if matches!(t, "pull" | "report" | "agent" | "ask" | "audit") {
        save(peers, &cfg.store, &board).await;
    }
    out
}

/// The board bundle: `pull`, `board`, `task`, `report`, `agent`, `ask`, `audit`.
#[derive(Default)]
pub struct BoardBundle;

#[async_trait::async_trait]
impl fantastic_kernel::bundle::Bundle for BoardBundle {
    fn name(&self) -> &str {
        HANDLER_MODULE
    }

    fn verb_docs(&self) -> Value {
        json!({
            "pull": "pull → {tasks: n, errors: [..]}",
            "board": "board → {tasks: {pending, in_work, done, failed}, agents: {..}}",
            "task": "task {id} → the task",
            "report": "report {task, status, note?, pr?, author?} → {ok}",
            "agent": "agent {name, state, cost} → {ok}",
            "ask": "ask {to, question} → the answer, recorded",
            "audit": "audit → {findings: [..]}, each asked of ask_to",
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
        let out = match cfg_of(kernel, agent_id) {
            Err(e) => json!({"error": e}),
            Ok(cfg) => dispatch(&KernelPeers(kernel.clone()), &cfg, t, payload).await,
        };
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            groups: BTreeMap::new(),
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
    fn board_rows_have_the_feed_shape() {
        let mut board = Board::default();
        board.add_issue("o/r", 1, "t", 0);
        board.apply_state("o/r#1", "in_work busy-w\n", 90);
        board.touch_agent("busy-w", "working", 1.5, 95);
        board.touch_agent("idle-w", "waiting", 0.0, 60);
        let mut live = BTreeMap::new();
        live.insert(
            "busy-w".to_string(),
            Live {
                generating: true,
                cache_hit: 0.8,
                ..Default::default()
            },
        );
        let rows = rows_from(&board, &BTreeMap::new(), &live, 100);
        assert_eq!(rows.len(), 2);
        let busy = rows.iter().find(|r| r["id"] == "busy-w").unwrap();
        assert_eq!(busy["state"], "busy");
        assert_eq!(busy["task"], "o/r#1");
        assert_eq!(busy["idle_s"], 5);
        assert_eq!(busy["time_on_task_s"], 10);
        let idle = rows.iter().find(|r| r["id"] == "idle-w").unwrap();
        assert_eq!(idle["state"], "idle");
        assert_eq!(idle["task"], "");
    }

    fn one_live(name: &str, lv: Live) -> BTreeMap<String, Live> {
        let mut m = BTreeMap::new();
        m.insert(name.to_string(), lv);
        m
    }

    #[test]
    fn a_member_with_an_open_inquiry_is_busy_with_that_task() {
        let mut board = Board::default();
        board.touch_agent("m1", "", 0.0, 90);
        let live = one_live(
            "m1",
            Live {
                inquiry: Some("do X\nmore".into()),
                newest_row: Some(97),
                ..Default::default()
            },
        );
        let rows = rows_from(&board, &BTreeMap::new(), &live, 100);
        let m = rows.iter().find(|r| r["id"] == "m1").unwrap();
        assert_eq!(m["state"], "busy");
        assert_eq!(m["task"], "do X");
        assert_eq!(m["idle_s"], 3);
    }

    #[test]
    fn a_claim_wins_over_an_inquiry_for_the_task() {
        let mut board = Board::default();
        board.add_issue("o/r", 1, "t", 0);
        board.apply_state("o/r#1", "in_work m1\n", 90);
        board.touch_agent("m1", "", 0.0, 90);
        let live = one_live(
            "m1",
            Live {
                inquiry: Some("do X".into()),
                ..Default::default()
            },
        );
        let rows = rows_from(&board, &BTreeMap::new(), &live, 100);
        let m = rows.iter().find(|r| r["id"] == "m1").unwrap();
        assert_eq!(m["task"], "o/r#1");
    }

    #[test]
    fn a_member_generating_is_busy() {
        let mut board = Board::default();
        board.touch_agent("m1", "", 0.0, 90);
        let live = one_live(
            "m1",
            Live {
                generating: true,
                ..Default::default()
            },
        );
        let rows = rows_from(&board, &BTreeMap::new(), &live, 100);
        let m = rows.iter().find(|r| r["id"] == "m1").unwrap();
        assert_eq!(m["state"], "busy");
        assert_eq!(m["task"], "");
    }

    #[test]
    fn an_inquiry_is_open_until_the_member_answers_it() {
        let inquiry = json!({"role": "user", "ts": 5.0,
            "content": "[inquiry from lead · re 7 — do any work it needs, then answer with <call name=\"inquiry_answer\"><answer>…</answer></call> before done: nothing else counts as your answer]\ndo X\nmore"});
        let other = json!({"role": "user", "ts": 6.0,
            "content": "[inquiry from user · re 8 — x]\nsomething else"});
        let working = json!({"role": "assistant", "ts": 7.0, "content": "reading files"});
        let answer = json!({"role": "assistant", "ts": 9.0,
            "content": "<call name=\"inquiry_answer\"><answer>ok</answer></call>"});
        let open = open_inquiry(&[inquiry.clone(), working.clone()], "lead");
        assert_eq!(open.as_deref().map(first_line).as_deref(), Some("do X"));
        // an inquiry from someone else is not the driver's
        assert_eq!(
            open_inquiry(&[inquiry.clone(), other], "lead").unwrap(),
            "do X\nmore"
        );
        assert_eq!(open_inquiry(&[inquiry, working, answer], "lead"), None);
        assert_eq!(open_inquiry(&[], "lead"), None);
        assert_eq!(
            newest_row(&[json!({"ts": 5.5}), json!({"ts": 9.0}), json!({})]),
            Some(9)
        );
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
    fn board_survives_restart() {
        let mut board = Board::default();
        board.add_issue("o/r", 1, "t", 0);
        board.apply_state("o/r#1", "in_work bob", 4);
        board.touch_agent("bob", "coding", 1.5, 4);
        let saved = serde_json::to_string(&board).unwrap();
        let reloaded: Board = serde_json::from_str(&saved).unwrap();
        assert_eq!(reloaded, board);
    }

    #[test]
    fn register_team_registers_board() {
        let reg = crate::registry();
        assert!(reg.get(HANDLER_MODULE).is_some());
    }

    #[test]
    fn rows_list_configured_groups_and_members() {
        let mut board = Board::default();
        board.touch_agent("m1", "working", 0.0, 90);
        let mut groups = BTreeMap::new();
        groups.insert(
            "g1".to_string(),
            Group {
                driver: "d1".into(),
                members: vec!["m1".into(), "m2".into()],
            },
        );
        let rows = rows_from(&board, &groups, &BTreeMap::new(), 100);
        let g = rows.iter().find(|r| r["id"] == "g1").unwrap();
        assert_eq!(g["parent"], Value::Null);
        assert_eq!(g["has_ai"], true);
        assert_eq!(g["driver"], "d1");
        let m1 = rows.iter().find(|r| r["id"] == "m1").unwrap();
        assert_eq!(m1["parent"], "g1");
    }

    #[test]
    fn a_member_never_registered_still_has_a_row() {
        let board = Board::default();
        let mut groups = BTreeMap::new();
        groups.insert(
            "g1".to_string(),
            Group {
                driver: String::new(),
                members: vec!["ghost".into()],
            },
        );
        let mut live = BTreeMap::new();
        live.insert(
            "ghost".to_string(),
            Live {
                generating: true,
                cache_hit: 0.0,
                ..Default::default()
            },
        );
        let rows = rows_from(&board, &groups, &live, 100);
        let m = rows.iter().find(|r| r["id"] == "ghost").unwrap();
        assert_eq!(m["parent"], "g1");
        assert_eq!(m["state"], "busy");
        let g = rows.iter().find(|r| r["id"] == "g1").unwrap();
        assert_eq!(g["has_ai"], false);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn team_groups_survive_a_real_kernel_create() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        let created = kernel
            .send(vec![fantastic_kernel::Item::new(
                "kernel",
                json!({
                    "type": "create_agent",
                    "id": "board_t",
                    "parent_id": "kernel_state",
                    "handler_module": HANDLER_MODULE,
                    "github": "gh",
                    "store": "store",
                    "team_groups": {
                        "g1": {"driver": "d1", "members": ["m1", "m2"]}
                    }
                }),
            )])
            .await;
        assert!(
            matches!(created[0].status, fantastic_kernel::Status::Completed),
            "create_agent failed: {:?}",
            created[0].result
        );
        let outcomes = kernel
            .send(vec![fantastic_kernel::Item::new(
                "board_t",
                json!({"type": "rows"}),
            )])
            .await;
        let rows = outcomes[0].result["rows"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let g = rows.iter().find(|r| r["id"] == "g1");
        assert!(g.is_some(), "group g1 missing from rows: {rows:?}");
        let m1 = rows.iter().find(|r| r["id"] == "m1").unwrap();
        assert_eq!(m1["parent"], "g1");
        kernel.shutdown().await;
        crate::bootstrap::shutdown(dir.path()).unwrap();
    }
}

#[cfg(test)]
mod load_tests {
    use super::*;

    const NOW: u64 = 10_000;

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

    /// One pending, two in work, three done (two with a PR), one failed.
    /// Group g1 has w1 (busy, fresh) and w2 (holds a task, silent 400s: stalled).
    /// Group g2 has w3 (idle, fresh).
    fn fixed_board() -> (Board, BTreeMap<String, Group>) {
        let mut b = Board::default();
        let rows = [
            task("o/r#1", Status::Pending, &[], None, NOW - 5),
            task("o/r#2", Status::InWork, &["w1"], None, NOW - 10),
            task("o/r#3", Status::InWork, &["w2"], None, NOW - 10),
            task("o/r#4", Status::Done, &["w1"], Some("p4"), NOW - 100),
            task("o/r#5", Status::Done, &["w1"], Some("p5"), NOW - 200),
            task("o/r#6", Status::Done, &["w3"], None, NOW - 4000),
            task("o/r#7", Status::Failed, &["w3"], None, NOW - 50),
        ];
        for t in rows {
            b.tasks.insert(t.id.clone(), t);
        }
        b.touch_agent("w1", "working", 0.0, NOW - 1);
        b.touch_agent("w2", "working", 0.0, NOW - 400);
        b.touch_agent("w3", "waiting", 0.0, NOW - 5);
        let mut groups = BTreeMap::new();
        groups.insert(
            "g1".to_string(),
            Group {
                driver: "d1".into(),
                members: vec!["w1".into(), "w2".into()],
            },
        );
        groups.insert(
            "g2".to_string(),
            Group {
                driver: String::new(),
                members: vec!["w3".into()],
            },
        );
        (b, groups)
    }

    #[test]
    fn load_counts_from_a_fixed_board() {
        let (b, groups) = fixed_board();
        let load = load_numbers(&b, &groups, &BTreeMap::new(), NOW, 300);
        assert_eq!(load["tasks"]["held"], 2);
        assert_eq!(load["tasks"]["queued"], 1);
        assert_eq!(load["tasks"]["done"], 3);
        assert_eq!(load["tasks"]["in_review"], 2);
        assert_eq!(load["tasks"]["blocked"], 1);
        assert_eq!(load["groups"]["g1"]["busy"], 1);
        assert_eq!(load["groups"]["g1"]["stalled"], 1);
        assert_eq!(load["groups"]["g1"]["idle"], 0);
        assert_eq!(load["groups"]["g2"]["idle"], 1);
        assert_eq!(load["groups"]["g2"]["busy"], 0);
    }

    #[test]
    fn stalled_worker_is_stalled_not_busy() {
        let (b, groups) = fixed_board();
        let load = load_numbers(&b, &groups, &BTreeMap::new(), NOW, 300);
        assert_eq!(load["workers"]["stalled"], 1);
        assert_eq!(load["workers"]["busy"], 1, "w1 holds a task but is fresh");
        assert_eq!(load["workers"]["idle"], 1);
        let w = load["workers"]["busy"].as_u64().unwrap()
            + load["workers"]["idle"].as_u64().unwrap()
            + load["workers"]["stalled"].as_u64().unwrap();
        assert_eq!(w, 3, "every worker is counted once");
    }

    #[test]
    fn throughput_counts_only_tasks_done_in_the_window() {
        let mut b = Board::default();
        b.tasks.insert(
            "o/r#1".into(),
            task("o/r#1", Status::Done, &["w1"], Some("p1"), NOW - 100),
        );
        b.tasks.insert(
            "o/r#2".into(),
            task("o/r#2", Status::Done, &["w1"], Some("p2"), NOW - 4000),
        );
        b.tasks.insert(
            "o/r#3".into(),
            task("o/r#3", Status::InWork, &["w1"], None, NOW - 10),
        );
        b.tasks.insert(
            "o/r#4".into(),
            task(
                "o/r#4",
                Status::Done,
                &["w1"],
                Some("p4"),
                NOW - LOAD_WINDOW_S,
            ),
        );
        let load = load_numbers(&b, &BTreeMap::new(), &BTreeMap::new(), NOW, 300);
        assert_eq!(load["done_last_hour"], 1);
    }

    #[test]
    fn load_numbers_has_integer_leaves_only() {
        fn walk(v: &Value, path: &str) {
            match v {
                Value::Object(m) => {
                    for (k, child) in m {
                        assert!(k != "total" && k != "of", "forbidden key {k} at {path}");
                        walk(child, &format!("{path}/{k}"));
                    }
                }
                Value::Number(n) => assert!(n.is_u64(), "non-integer at {path}: {n}"),
                // A group's driver id is the one string: data, empty for none.
                Value::String(_) if path.ends_with("/driver") => {}
                other => panic!("non-integer leaf at {path}: {other}"),
            }
        }
        let (b, groups) = fixed_board();
        let load = load_numbers(&b, &groups, &BTreeMap::new(), NOW, 300);
        walk(&load, "");
    }

    /// A board for the token tests: group g1 has driver `d` and member `w`, the
    /// driverless group g2 has member `x`. `d` is not a member of any group.
    fn token_groups() -> BTreeMap<String, Group> {
        let mut groups = BTreeMap::new();
        groups.insert(
            "g1".to_string(),
            Group {
                driver: "d".into(),
                members: vec!["w".into()],
            },
        );
        groups.insert(
            "g2".to_string(),
            Group {
                driver: String::new(),
                members: vec!["x".into()],
            },
        );
        groups
    }

    #[test]
    fn load_numbers_tokens_total_counts_only_group_drivers() {
        let groups = token_groups();
        let mut live = BTreeMap::new();
        live.insert(
            "d".to_string(),
            Live {
                tokens: 500,
                ..Live::default()
            },
        );
        live.insert(
            "w".to_string(),
            Live {
                tokens: 900,
                ..Live::default()
            },
        );
        live.insert(
            "x".to_string(),
            Live {
                tokens: 40,
                ..Live::default()
            },
        );
        let load = load_numbers(&Board::default(), &groups, &live, NOW, 300);
        assert_eq!(load["tokens_total"], 500);
    }

    /// Answers the board's reads: each agent's `reflect` usage, and no store file.
    struct TokenPeers;

    #[async_trait::async_trait]
    impl Peers for TokenPeers {
        async fn ask(&self, to: &str, p: Value) -> Value {
            match (to, p["type"].as_str().unwrap_or("")) {
                ("store", "read") => json!({"error": "not found"}),
                ("d", "reflect") => json!({"usage": {"prompt_tokens": 300, "output_tokens": 200}}),
                ("w", "reflect") => json!({"usage": {"prompt_tokens": 600, "output_tokens": 300}}),
                (_, "rows") => json!({"rows": []}),
                _ => json!({"reply": "noted"}),
            }
        }
    }

    #[tokio::test]
    async fn rows_handler_reads_the_driver_even_if_not_a_member() {
        let cfg = BoardCfg {
            repos: Vec::new(),
            github: "gh".into(),
            store: "store".into(),
            ask_to: "user".into(),
            stale_secs: 1800,
            groups: token_groups(),
        };
        let out = dispatch(&TokenPeers, &cfg, "rows", &json!({"type": "rows"})).await;
        assert_eq!(out["load"]["tokens_total"], 500);
    }
}
