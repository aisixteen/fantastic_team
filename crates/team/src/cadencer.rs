//! The cadencer (`team_cadencer.tools`): on a cadence each lead gets a short
//! digest of the level directly below it. A sub-lead's line rolls up its whole
//! subtree. Only changes since the last digest plus every stall are sent, and an
//! empty digest is not sent. A lead with no AI gets its digest on the board.
//!
//! The builder is pure: it takes a snapshot of the tree and the previous
//! snapshot. The bundle returns the digests; delivering them into each lead's
//! conversation is the caller's job. The bundle runs its own timer (when enabled)
//! and beats with the last tree given by a `tick`.
//!
//! Quiet mode still sends a short heartbeat digest every `HEARTBEAT_EVERY` beats
//! to a lead that has an idle member while work waits in the queue, so a driver
//! never sleeps through queued work. Each group of `team.yaml` may set its own
//! `cadence`, `stall_after`, `quiet` and `enabled`; an unset key follows the
//! global setting (changed live by `set`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fantastic_kernel::bundle::{Bundle, BundleError, Reply};
use fantastic_kernel::{AgentId, Kernel};
use serde_json::{json, Value};
use tokio::task::JoinHandle;

use crate::cadence_feed::{self, CostMeter};
use crate::puller::{KernelPeers, Peers};

/// The handler module name this bundle registers under.
pub const HANDLER_MODULE: &str = "team_cadencer.tools";
const NAME: &str = "team_cadencer";
const BOARD: &str = "board";

/// One agent in a snapshot of the tree.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Node {
    pub(crate) id: String,
    pub(crate) parent: Option<String>,
    pub(crate) driver: String,
    pub(crate) has_ai: bool,
    pub(crate) state: String,
    pub(crate) task: String,
    pub(crate) open_inquiry: bool,
    pub(crate) idle_s: u64,
    pub(crate) time_on_task_s: u64,
    pub(crate) cost: f64,
    pub(crate) cache_hit: f64,
    pub(crate) new_results: u64,
}

/// The live settings of the cadencer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Settings {
    pub(crate) enabled: bool,
    pub(crate) cadence_s: u64,
    pub(crate) stall_after_s: u64,
    pub(crate) quiet: bool,
    pub(crate) off: BTreeSet<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            enabled: true,
            cadence_s: 60,
            stall_after_s: 300,
            quiet: true,
            off: BTreeSet::new(),
        }
    }
}

/// What one group of `team.yaml` overrides, by the group's name (the lead's id).
/// An unset key follows the global setting.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct GroupSettings {
    pub(crate) cadence_s: Option<u64>,
    pub(crate) stall_after_s: Option<u64>,
    pub(crate) quiet: Option<bool>,
    pub(crate) enabled: Option<bool>,
}

type Groups = BTreeMap<String, GroupSettings>;

/// Beats between two heartbeat digests of a quiet lead with idle members and queued work.
const HEARTBEAT_EVERY: u64 = 3;

/// The settings that count for one lead: the group's override, else the global value.
/// `enabled` is read apart (see [`group_disabled`]), since the global one is the timer's.
fn effective(s: &Settings, groups: &Groups, lead: &str) -> Settings {
    let mut e = s.clone();
    if let Some(g) = groups.get(lead) {
        if let Some(v) = g.stall_after_s {
            e.stall_after_s = v;
        }
        if let Some(v) = g.quiet {
            e.quiet = v;
        }
        if let Some(v) = g.cadence_s {
            e.cadence_s = v;
        }
    }
    e
}

/// True when the group switched the cadencer off for itself in `team.yaml`.
fn group_disabled(groups: &Groups, lead: &str) -> bool {
    groups.get(lead).and_then(|g| g.enabled) == Some(false)
}

/// One digest: who it goes to and its lines.
#[derive(Debug, PartialEq)]
pub(crate) struct Digest {
    pub(crate) to: String,
    pub(crate) lines: Vec<String>,
}

fn children_of<'a>(tree: &'a [Node], id: &str) -> Vec<&'a Node> {
    tree.iter()
        .filter(|n| n.parent.as_deref() == Some(id))
        .collect()
}

fn subtree<'a>(tree: &'a [Node], id: &str) -> Vec<&'a Node> {
    let mut out = Vec::new();
    let mut stack = children_of(tree, id);
    while let Some(n) = stack.pop() {
        out.push(n);
        stack.extend(children_of(tree, &n.id));
    }
    out
}

/// A node holds a task when it has one, or an inquiry is open on it.
fn holds_task(n: &Node) -> bool {
    !n.task.is_empty() || n.open_inquiry
}

/// Stalled only while holding a task and idle past the threshold (the board's
/// one definition, shared with the load counts).
fn stalled(n: &Node, s: &Settings) -> bool {
    crate::board::is_stalled(holds_task(n), n.idle_s, s.stall_after_s)
}

fn changed(now: &Node, prev: Option<&Node>) -> bool {
    match prev {
        None => true,
        Some(p) => {
            p.state != now.state
                || p.task != now.task
                || p.cost != now.cost
                || p.new_results != now.new_results
        }
    }
}

fn node_line(n: &Node, s: &Settings) -> String {
    let new = if n.new_results > 0 {
        format!(" · +{} new", n.new_results)
    } else {
        String::new()
    };
    let stall = if stalled(n, s) {
        format!(" · STALLED {}s", n.idle_s)
    } else {
        String::new()
    };
    format!(
        "{}: {} · {} · {}s on task · cost {:.2} · cache {:.0}%{}{}",
        n.id,
        n.state,
        n.task,
        n.time_on_task_s,
        n.cost,
        n.cache_hit * 100.0,
        new,
        stall
    )
}

fn rollup_line(id: &str, group: &[&Node], s: &Settings) -> String {
    let mut by: BTreeMap<&str, usize> = BTreeMap::new();
    for n in group {
        *by.entry(n.state.as_str()).or_insert(0) += 1;
    }
    let states = by
        .iter()
        .map(|(k, v)| format!("{v} {k}"))
        .collect::<Vec<_>>()
        .join(", ");
    let new: u64 = group.iter().map(|n| n.new_results).sum();
    let stalls = group.iter().filter(|n| stalled(n, s)).count();
    let stall = if stalls > 0 {
        format!(" · {stalls} STALLED")
    } else {
        String::new()
    };
    format!(
        "{id}: {} agents ({states}) · +{new} new{stall}",
        group.len()
    )
}

/// Build the digests for one tick. A line goes out when its branch changed
/// since `prev`, or when a worker in it is stalled; in quiet mode nothing else
/// does. A lead with no lines gets no digest.
#[cfg(test)]
pub(crate) fn build(tree: &[Node], prev: &[Node], s: &Settings) -> Vec<Digest> {
    build_with(tree, prev, s, &Groups::new())
        .into_iter()
        .map(|(_, d)| d)
        .collect()
}

/// Where a lead's digest goes: its driver, else the lead itself when it has an AI, else the board.
fn target_of(lead: &Node) -> String {
    if !lead.driver.is_empty() {
        lead.driver.clone()
    } else if lead.has_ai {
        lead.id.clone()
    } else {
        BOARD.to_string()
    }
}

/// [`build`] with the per-group settings; each digest comes with its lead's id.
fn build_with(
    tree: &[Node],
    prev: &[Node],
    global: &Settings,
    groups: &Groups,
) -> Vec<(String, Digest)> {
    let mut out = Vec::new();
    for lead in tree.iter().filter(|n| !children_of(tree, &n.id).is_empty()) {
        if global.off.contains(&lead.id) || group_disabled(groups, &lead.id) {
            continue;
        }
        let s = &effective(global, groups, &lead.id);
        let mut lines = Vec::new();
        for c in children_of(tree, &lead.id) {
            let mut group: Vec<&Node> = vec![c];
            group.extend(subtree(tree, &c.id));
            let any_change = group
                .iter()
                .any(|n| changed(n, prev.iter().find(|p| p.id == n.id)));
            let any_stall = group.iter().any(|n| stalled(n, s));
            if s.quiet && !any_change && !any_stall {
                continue;
            }
            lines.push(if group.len() == 1 {
                node_line(c, s)
            } else {
                rollup_line(&c.id, &group, s)
            });
        }
        if lines.is_empty() {
            continue;
        }
        out.push((
            lead.id.clone(),
            Digest {
                to: target_of(lead),
                lines,
            },
        ));
    }
    out
}

/// The quiet-mode heartbeat. A quiet lead that got no digest this beat, has an
/// idle member (no task in hand) and sees work queued counts a beat in
/// `counters`; every `HEARTBEAT_EVERY`th such beat it gets one short line. The
/// counter restarts when the lead got a regular digest or the condition fails.
/// `got` holds this beat's digests with their lead; the new ones are returned.
fn heartbeat(
    tree: &[Node],
    got: &[(String, Digest)],
    load: &Value,
    global: &Settings,
    groups: &Groups,
    counters: &mut BTreeMap<String, u64>,
) -> Vec<(String, Digest)> {
    let queued = load["tasks"]["queued"].as_u64().unwrap_or(0);
    let mut out = Vec::new();
    for lead in tree.iter().filter(|n| !children_of(tree, &n.id).is_empty()) {
        let s = effective(global, groups, &lead.id);
        let idle = children_of(tree, &lead.id)
            .iter()
            .filter(|c| c.state == "idle" && !holds_task(c))
            .count();
        let due = s.quiet
            && !global.off.contains(&lead.id)
            && !group_disabled(groups, &lead.id)
            && !got.iter().any(|(l, _)| *l == lead.id)
            && idle > 0
            && queued > 0;
        if !due {
            counters.remove(&lead.id);
            continue;
        }
        let c = counters.entry(lead.id.clone()).or_insert(0);
        *c += 1;
        if *c >= HEARTBEAT_EVERY {
            *c = 0;
            out.push((
                lead.id.clone(),
                Digest {
                    to: target_of(lead),
                    lines: vec![format!("heartbeat: {idle} idle, {queued} queued")],
                },
            ));
        }
    }
    out
}

fn str_of(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn u64_of(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(|x| x.as_u64()).unwrap_or(0)
}

fn node_from(v: &Value) -> Option<Node> {
    let id = v.get("id")?.as_str()?.to_string();
    Some(Node {
        id,
        parent: v.get("parent").and_then(|x| x.as_str()).map(String::from),
        has_ai: v.get("has_ai").and_then(|x| x.as_bool()).unwrap_or(true),
        driver: str_of(v, "driver"),
        state: str_of(v, "state"),
        task: str_of(v, "task"),
        open_inquiry: v.get("inquiry").and_then(|x| x.as_bool()).unwrap_or(false),
        idle_s: u64_of(v, "idle_s"),
        time_on_task_s: u64_of(v, "time_on_task_s"),
        cost: v.get("cost").and_then(|x| x.as_f64()).unwrap_or(0.0),
        cache_hit: v.get("cache_hit").and_then(|x| x.as_f64()).unwrap_or(0.0),
        new_results: u64_of(v, "new_results"),
    })
}

fn apply_set(s: &mut Settings, p: &Value) {
    if let Some(e) = p.get("enabled").and_then(|x| x.as_bool()) {
        s.enabled = e;
    }
    if let Some(c) = p.get("cadence").and_then(|x| x.as_u64()) {
        s.cadence_s = c;
    }
    if let Some(c) = p.get("stall_after").and_then(|x| x.as_u64()) {
        s.stall_after_s = c;
    }
    if let Some(q) = p.get("quiet").and_then(|x| x.as_bool()) {
        s.quiet = q;
    }
    if let Some(lead) = p.get("lead").and_then(|x| x.as_str()) {
        let on = p.get("on").and_then(|x| x.as_bool()).unwrap_or(true);
        if on {
            s.off.remove(lead);
        } else {
            s.off.insert(lead.to_string());
        }
    }
}

fn group_settings_json(groups: &Groups) -> Value {
    let mut out = serde_json::Map::new();
    for (g, v) in groups {
        let mut m = serde_json::Map::new();
        if let Some(x) = v.cadence_s {
            m.insert("cadence".into(), json!(x));
        }
        if let Some(x) = v.stall_after_s {
            m.insert("stall_after".into(), json!(x));
        }
        if let Some(x) = v.quiet {
            m.insert("quiet".into(), json!(x));
        }
        if let Some(x) = v.enabled {
            m.insert("enabled".into(), json!(x));
        }
        out.insert(g.clone(), Value::Object(m));
    }
    Value::Object(out)
}

fn groups_from_json(v: &Value) -> Groups {
    let mut out = Groups::new();
    for (g, o) in v.as_object().into_iter().flatten() {
        out.insert(
            g.clone(),
            GroupSettings {
                cadence_s: o.get("cadence").and_then(Value::as_u64),
                stall_after_s: o.get("stall_after").and_then(Value::as_u64),
                quiet: o.get("quiet").and_then(Value::as_bool),
                enabled: o.get("enabled").and_then(Value::as_bool),
            },
        );
    }
    out
}

fn settings_json(s: &Settings) -> Value {
    json!({
        "enabled": s.enabled,
        "cadence": s.cadence_s,
        "stall_after": s.stall_after_s,
        "quiet": s.quiet,
        "off": s.off.iter().collect::<Vec<_>>(),
    })
}

/// The record keys the settings and board are kept under: the verb arg names.
/// Written into the cadencer's own agent record on `set` and `configure`, and
/// read back at boot.
fn record_patch(inner: &Inner) -> serde_json::Map<String, Value> {
    let mut m = match settings_json(&inner.settings) {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    if let Some(b) = &inner.board {
        m.insert("board".to_string(), json!(b));
    }
    m.insert(
        "group_settings".to_string(),
        group_settings_json(&inner.groups),
    );
    // `parent` is a reserved record key, so the cadencer's parent has its own.
    if let Some(p) = &inner.parent {
        m.insert("cadencer_parent".to_string(), json!(p));
    }
    m
}

/// Load the keys `record_patch` writes from a record. Returns true when any
/// of them was present.
fn load_from_record(inner: &mut Inner, meta: &serde_json::Map<String, Value>) -> bool {
    let mut loaded = false;
    if let Some(b) = meta.get("board").and_then(|v| v.as_str()) {
        inner.board = Some(b.to_string());
        loaded = true;
    }
    if let Some(r) = meta.get("roles").and_then(|v| v.as_str()) {
        inner.roles = Some(r.to_string());
        loaded = true;
    }
    if let Some(p) = meta.get("cadencer_parent").and_then(|v| v.as_str()) {
        inner.parent = Some(p.to_string());
        loaded = true;
    }
    if let Some(g) = meta.get("group_settings").filter(|v| v.is_object()) {
        inner.groups = groups_from_json(g);
        loaded = true;
    }
    let mut settings = serde_json::Map::new();
    for k in ["enabled", "cadence", "stall_after", "quiet"] {
        if let Some(v) = meta.get(k) {
            settings.insert(k.to_string(), v.clone());
            loaded = true;
        }
    }
    apply_set(&mut inner.settings, &Value::Object(settings));
    if let Some(arr) = meta.get("off").and_then(|v| v.as_array()) {
        inner.settings.off = arr
            .iter()
            .filter_map(|x| x.as_str())
            .map(String::from)
            .collect();
        loaded = true;
    }
    loaded
}

fn digest_json(d: &Digest) -> Value {
    json!({"to": d.to, "lines": d.lines, "text": d.lines.join("\n")})
}

#[derive(Default)]
struct Inner {
    settings: Settings,
    parent: Option<String>,
    tree: Vec<Node>,
    prev: Vec<Node>,
    ticks: u64,
    digests_sent: u64,
    delivery_errors: u64,
    meter: CostMeter,
    peers: Option<Arc<dyn Peers>>,
    board: Option<String>,
    /// The roles registry (roles mode): the digest tree is the spawned tree, read from it.
    roles: Option<String>,
    /// The kernel, for the grants to the spawned leads (set with the peers).
    kernel: Option<Arc<Kernel>>,
    /// The groups' own settings from `team.yaml`, by group name.
    groups: Groups,
    /// Consecutive quiet beats with idle members and queued work, per lead.
    quiet_beats: BTreeMap<String, u64>,
    /// The clock second of each lead's last digest.
    last_digest_s: BTreeMap<String, u64>,
    timer_cfg: Option<(bool, u64)>,
    /// The board's `load` object from the last read (`Null` when none came).
    load: Value,
    /// The cumulative token count at the last beat; the beat records the change.
    tokens_seen: u64,
    /// The clock's zero, set at first use. Per cadencer, so one cadencer's
    /// clock never depends on another's (tokio paused time differs per test).
    origin: Option<tokio::time::Instant>,
    last_beat_s: Option<u64>,
    next_beat_s: Option<u64>,
    timer: Option<JoinHandle<()>>,
}

impl Inner {
    /// Seconds since this cadencer's first use; honors tokio paused time.
    fn clock(&mut self) -> u64 {
        let origin = *self.origin.get_or_insert_with(tokio::time::Instant::now);
        tokio::time::Instant::now().duration_since(origin).as_secs()
    }
}

fn timer_alive(inner: &Inner) -> bool {
    inner.timer.as_ref().is_some_and(|h| !h.is_finished())
}

/// One beat: build digests from the stored tree, then remember it.
fn beat(inner: &mut Inner) -> Vec<Digest> {
    let now = inner.clock();
    // A group whose own cadence is longer than the timer's is skipped until it is due.
    let mut s = inner.settings.clone();
    let mut skipped = Vec::new();
    for (lead, g) in &inner.groups {
        let due = g.cadence_s.is_none_or(|c| {
            c <= s.cadence_s
                || inner
                    .last_digest_s
                    .get(lead)
                    .is_none_or(|last| now.saturating_sub(*last) >= c)
        });
        if !due {
            s.off.insert(lead.clone());
            skipped.push(lead.clone());
        }
    }
    let mut got = build_with(&inner.tree, &inner.prev, &s, &inner.groups);
    let beats = heartbeat(
        &inner.tree,
        &got,
        &inner.load,
        &s,
        &inner.groups,
        &mut inner.quiet_beats,
    );
    got.extend(beats);
    for (lead, _) in &got {
        inner.last_digest_s.insert(lead.clone(), now);
    }
    let mut digests: Vec<Digest> = got.into_iter().map(|(_, d)| d).collect();
    // The driver's tokens: the change of the board's cumulative counter since
    // the last beat goes into the meter (the first beat counts from zero).
    if let Some(total) = inner.load["tokens_total"].as_u64() {
        // A lower total means the counter restarted: the new total is all new.
        let delta = if total >= inner.tokens_seen {
            total - inner.tokens_seen
        } else {
            total
        };
        inner.tokens_seen = total;
        inner.meter.record_tokens(now, delta);
    }
    let tok = inner.meter.status(now)["driver_tokens_per_hour"]
        .as_u64()
        .unwrap_or(0);
    if let Some(line) = cadence_feed::load_line(&inner.load, tok) {
        for d in digests.iter_mut() {
            d.lines.push(line.clone());
        }
    }
    // What a skipped group did not see stays "changed" for its next due beat.
    let mut next_prev = inner.tree.clone();
    for lead in &skipped {
        let mut ids: Vec<String> = subtree(&inner.tree, lead)
            .iter()
            .map(|n| n.id.clone())
            .collect();
        ids.sort();
        for id in ids {
            match inner.prev.iter().find(|p| p.id == id) {
                Some(old) => {
                    if let Some(slot) = next_prev.iter_mut().find(|p| p.id == id) {
                        *slot = old.clone();
                    }
                }
                None => next_prev.retain(|p| p.id != id),
            }
        }
    }
    inner.prev = next_prev;
    inner.ticks += 1;
    inner.digests_sent += digests.len() as u64;
    inner.last_beat_s = Some(now);
    inner.next_beat_s = if timer_alive(inner) {
        Some(now + inner.settings.cadence_s)
    } else {
        None
    };
    digests
}

/// Ask the board for its rows and turn them into the tree shape, with the
/// board's `load` object (`Null` when the reply has none). `None` when the
/// board gives no `rows` list.
async fn read_board(
    peers: &dyn Peers,
    board: &str,
    stall_after_s: u64,
) -> Option<(Vec<Node>, Value)> {
    let reply = peers
        .ask(
            board,
            json!({"type": "rows", "stall_after_s": stall_after_s}),
        )
        .await;
    let body = if reply.get("rows").is_some() {
        reply
    } else {
        reply.get("result")?.clone()
    };
    let rows = body.get("rows")?.as_array()?;
    let nodes = cadence_feed::tree_from_board(rows)
        .iter()
        .filter_map(node_from)
        .collect();
    let load = body.get("load").cloned().unwrap_or(Value::Null);
    Some((nodes, load))
}

/// The spawned tree (roles mode) as nodes, with its load: the registries' reads
/// through the public verbs only ([`crate::board::roles_tree_rows`]). `None` when the
/// project_lead's branch could not be read, so the last tree stays.
async fn read_roles(
    peers: &dyn Peers,
    roles: &str,
    stall_after_s: u64,
) -> Option<(Vec<Node>, Value)> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (rows, load) = crate::board::roles_tree_rows(peers, roles, now, stall_after_s).await;
    if rows.is_empty() {
        return None;
    }
    Some((rows.iter().filter_map(node_from).collect(), load))
}

/// Where the tree comes from: the spawned tree when the team has `roles:`, else the
/// board's rows (a team without `roles:` keeps the old path unchanged).
async fn read_source(
    peers: &dyn Peers,
    board: Option<&str>,
    roles: Option<&str>,
    stall_after_s: u64,
) -> Option<(Vec<Node>, Value)> {
    match (roles, board) {
        (Some(r), _) => read_roles(peers, r, stall_after_s).await,
        (None, Some(b)) => read_board(peers, b, stall_after_s).await,
        (None, None) => None,
    }
}

/// Send each digest to its target. A failed send is counted, never fatal.
/// The lock is not held across the await.
async fn deliver(shared: &Arc<Mutex<Inner>>, peers: Option<Arc<dyn Peers>>, digests: Vec<Digest>) {
    for d in digests {
        let text = d.lines.join("\n");
        let failed = match &peers {
            None => true,
            Some(p) => p
                .ask(&d.to, json!({"type": "send", "text": text}))
                .await
                .get("error")
                .is_some(),
        };
        let mut g = shared.lock().expect("cadencer state");
        if failed {
            g.delivery_errors += 1;
        } else {
            let now = g.clock();
            g.meter.record_digest(now, text.len() as u64);
        }
    }
}

/// One timer beat: refresh the tree from its source, build the digests and send them.
/// In roles mode the cadencer is first let `send` to the leads the registry lists now,
/// so a project_lead spawned after the start gets its digest too.
async fn beat_once(shared: &Arc<Mutex<Inner>>) {
    let (peers, board, roles, kernel, stall_after_s) = {
        let g = shared.lock().expect("cadencer state");
        (
            g.peers.clone(),
            g.board.clone(),
            g.roles.clone(),
            g.kernel.clone(),
            g.settings.stall_after_s,
        )
    };
    if let Some(p) = peers.as_ref() {
        if let (Some(k), Some(r)) = (kernel.as_ref(), roles.as_deref()) {
            crate::tree::grant_fleet_digest_targets(k, r).await;
        }
        if let Some((tree, load)) = read_source(
            p.as_ref(),
            board.as_deref(),
            roles.as_deref(),
            stall_after_s,
        )
        .await
        {
            let mut g = shared.lock().expect("cadencer state");
            g.tree = tree;
            g.load = load;
        }
    }
    let (digests, peers) = {
        let mut g = shared.lock().expect("cadencer state");
        let digests = beat(&mut g);
        (digests, g.peers.clone())
    };
    deliver(shared, peers, digests).await;
}

/// Stop the old timer and, when enabled with a cadence, start a new one.
fn restart_timer(shared: &Arc<Mutex<Inner>>, inner: &mut Inner) {
    inner.timer_cfg = Some((inner.settings.enabled, inner.settings.cadence_s));
    if let Some(h) = inner.timer.take() {
        h.abort();
    }
    inner.next_beat_s = None;
    if !inner.settings.enabled || inner.settings.cadence_s == 0 {
        return;
    }
    let period = Duration::from_secs(inner.settings.cadence_s);
    let shared = Arc::clone(shared);
    inner.timer = Some(tokio::spawn(async move {
        let mut every = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        loop {
            every.tick().await;
            beat_once(&shared).await;
        }
    }));
    let now = inner.clock();
    inner.next_beat_s = Some(now + inner.settings.cadence_s);
}

/// Only the parent, `maintainer`, `user` or the CLI leg (`mcp`) may steer the cadencer. A send with no origin is an
/// operator send, which is trusted.
fn allowed(inner: &Inner, origin: Option<&str>) -> bool {
    match origin {
        None => true,
        Some(s) => {
            s == "maintainer" || s == "user" || s == "mcp" || inner.parent.as_deref() == Some(s)
        }
    }
}

fn dispatch(shared: &Arc<Mutex<Inner>>, payload: &Value) -> Value {
    let t = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let mut inner = shared.lock().expect("cadencer state");
    match t {
        "configure" => {
            if let Some(b) = payload.get("board").and_then(|v| v.as_str()) {
                inner.board = Some(b.to_string());
            }
            if let Some(p) = payload.get("parent").and_then(|v| v.as_str()) {
                inner.parent = Some(p.to_string());
            }
            restart_timer(shared, &mut inner);
            json!({"parent": inner.parent})
        }
        "tick" => {
            inner.tree = payload
                .get("tree")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(node_from).collect())
                .unwrap_or_default();
            let digests = beat(&mut inner);
            json!({
                "digests": digests.iter().map(digest_json).collect::<Vec<_>>(),
                "sent": digests.len(),
            })
        }
        "set" => {
            let seen = fantastic_kernel::current_origin().map(|a| a.as_str().to_string());
            if !allowed(&inner, seen.as_deref()) {
                return json!({"error": format!(
                    "team_cadencer: set refused: sender {}, parent {}",
                    seen.as_deref().unwrap_or("operator"),
                    inner.parent.as_deref().unwrap_or("none")
                )});
            }
            apply_set(&mut inner.settings, payload);
            let want = (inner.settings.enabled, inner.settings.cadence_s);
            if inner.timer_cfg != Some(want) {
                restart_timer(shared, &mut inner);
            }
            settings_json(&inner.settings)
        }
        "status" => {
            let now = inner.clock();
            let meter = inner.meter.status(now);
            // The board's load counts, plus the cadencer's own per-hour figures.
            let mut load = inner.load.as_object().cloned().unwrap_or_default();
            load.insert(
                "digest_bytes_per_hour".into(),
                meter["digest_bytes_per_hour"].clone(),
            );
            load.insert(
                "driver_tokens_per_hour".into(),
                meter["driver_tokens_per_hour"].clone(),
            );
            json!({
                "settings": settings_json(&inner.settings),
                "ticks": inner.ticks,
                "last_beat_s": inner.last_beat_s,
                "next_beat_s": inner.next_beat_s,
                "digests_sent": inner.digests_sent,
                "delivery_errors": inner.delivery_errors,
                "digest_bytes_per_hour": meter["digest_bytes_per_hour"],
                "driver_tokens_per_hour": meter["driver_tokens_per_hour"],
                "load": load,
            })
        }
        other => json!({"error": format!("team_cadencer: unknown type '{other}'")}),
    }
}

/// The cadencer bundle.
#[derive(Default)]
pub struct CadencerBundle {
    inner: Arc<Mutex<Inner>>,
}

impl CadencerBundle {
    /// Boot: load the saved keys from the agent record, then restart the timer
    /// when it is enabled and has a board.
    fn restore(&self, agent_id: &AgentId, kernel: &Arc<Kernel>) -> Value {
        let meta = kernel
            .resolve_target(agent_id.as_str())
            .map(|a| a.meta.read().expect("meta lock poisoned").clone())
            .unwrap_or_default();
        json!({"loaded": boot_from_meta(&self.inner, &meta)})
    }
}

/// Load the saved keys and restart the timer when it is enabled with a board or roles.
/// Returns true when the record had any of them.
fn boot_from_meta(shared: &Arc<Mutex<Inner>>, meta: &serde_json::Map<String, Value>) -> bool {
    let mut g = shared.lock().expect("cadencer state");
    let loaded = load_from_record(&mut g, meta);
    if g.settings.enabled && (g.board.is_some() || g.roles.is_some()) {
        restart_timer(shared, &mut g);
    }
    loaded
}

#[async_trait::async_trait]
impl Bundle for CadencerBundle {
    fn name(&self) -> &str {
        NAME
    }

    fn verb_docs(&self) -> Value {
        json!({
            "tick": "tick {tree: [{id, parent?, has_ai?, state, task, inquiry?, idle_s, time_on_task_s, cost, cache_hit, new_results}]} → {digests: [{to, lines, text}], sent}",
            "set": "set {sender, enabled?, cadence?, stall_after?, quiet?, lead?, on?} → settings (sender must be the parent, maintainer, user, the CLI or the dispatcher page); persisted in the agent record and restored at boot",
            "configure": "configure {board?, parent?} → {parent}; board and parent (kept as cadencer_parent) are persisted in the agent record and restored at boot, and the timer restarts",
            "boot": "the kernel's lifecycle verb, boot {} → {loaded}: loads the settings and board from the agent record and restarts the timer when enabled with a board or roles",
            "status": "status {} → {settings, ticks, last_beat_s, next_beat_s, digests_sent, digest_bytes_per_hour, driver_tokens_per_hour, load}; load is the board's counts (workers busy/idle/stalled per group, tasks held/in_review/done/blocked/queued, done_last_hour, tokens_total) and carries no mode",
        })
    }

    async fn handle(
        &self,
        agent_id: &AgentId,
        payload: &Value,
        kernel: &Arc<Kernel>,
    ) -> Result<Reply, BundleError> {
        {
            let mut g = self.inner.lock().expect("cadencer state");
            g.peers = Some(Arc::new(KernelPeers(kernel.clone())));
            g.kernel = Some(kernel.clone());
        }
        let t = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if t == "boot" {
            return Ok(Some(self.restore(agent_id, kernel)));
        }
        let reply = dispatch(&self.inner, payload);
        if matches!(t, "configure" | "set") && reply.get("error").is_none() {
            // The lock is dropped before the record write.
            let patch = {
                let g = self.inner.lock().expect("cadencer state");
                record_patch(&g)
            };
            let _ = kernel.update(agent_id, patch);
        }
        Ok(Some(reply))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(id: &str, parent: Option<&str>, has_ai: bool, state: &str, idle: u64) -> Node {
        Node {
            id: id.into(),
            parent: parent.map(String::from),
            driver: String::new(),
            has_ai,
            state: state.into(),
            task: "t".into(),
            open_inquiry: false,
            idle_s: idle,
            time_on_task_s: 10,
            cost: 0.5,
            cache_hit: 0.8,
            new_results: 0,
        }
    }

    /// root → A, B (sub-leads) → three workers each.
    fn two_by_three() -> Vec<Node> {
        let mut t = vec![
            n("root", None, true, "busy", 0),
            n("A", Some("root"), true, "busy", 0),
            n("B", Some("root"), true, "busy", 0),
        ];
        for lead in ["A", "B"] {
            for w in 1..=3 {
                t.push(n(&format!("{lead}{w}"), Some(lead), true, "busy", 0));
            }
        }
        t
    }

    fn digest_for<'a>(ds: &'a [Digest], to: &str) -> Option<&'a Digest> {
        ds.iter().find(|d| d.to == to)
    }

    #[test]
    fn root_gets_one_rollup_line_per_sublead() {
        let t = two_by_three();
        let ds = build(&t, &[], &Settings::default());
        let root = digest_for(&ds, "root").expect("root digest");
        assert_eq!(root.lines.len(), 2);
        assert!(root.lines[0].starts_with("A:") && root.lines[0].contains("4 agents"));
        assert!(root.lines[1].starts_with("B:") && root.lines[1].contains("4 agents"));
    }

    #[test]
    fn sublead_gets_one_line_per_worker() {
        let t = two_by_three();
        let ds = build(&t, &[], &Settings::default());
        assert_eq!(digest_for(&ds, "A").unwrap().lines.len(), 3);
        assert_eq!(digest_for(&ds, "B").unwrap().lines.len(), 3);
    }

    #[test]
    fn stalled_worker_reported_after_stall_after_once() {
        let mut t = two_by_three();
        let prev = t.clone();
        // Below the threshold: nothing reported in quiet mode.
        t[3].idle_s = 299;
        let ds = build(&t, &prev, &Settings::default());
        assert!(digest_for(&ds, "A").is_none());
        // Past the threshold: reported, one line for that worker.
        t[3].idle_s = 300;
        let ds = build(&t, &prev, &Settings::default());
        let a = digest_for(&ds, "A").expect("A digest");
        assert_eq!(a.lines.len(), 1);
        assert!(a.lines[0].contains("A1") && a.lines[0].contains("STALLED"));
    }

    #[test]
    fn still_stalled_worker_reported_again_next_digest() {
        let mut t = two_by_three();
        t[3].idle_s = 400;
        let prev = t.clone();
        let first = build(&t, &prev, &Settings::default());
        let second = build(&t, &prev, &Settings::default());
        assert!(digest_for(&first, "A").unwrap().lines[0].contains("STALLED"));
        assert!(digest_for(&second, "A").unwrap().lines[0].contains("STALLED"));
    }

    #[test]
    fn idle_member_without_a_task_is_not_stalled() {
        let mut reviewer = n("reviewer", Some("L"), true, "idle", 600);
        reviewer.task = String::new();
        let t = vec![n("L", None, true, "busy", 0), reviewer];
        let ds = build(&t, &[], &Settings::default());
        for d in &ds {
            for line in &d.lines {
                assert!(!line.contains("STALLED"), "unexpected: {line}");
            }
        }
    }

    #[test]
    fn live_set_takes_effect_next_tick() {
        let mut bundle = Inner::default();
        let mut t = two_by_three();
        t[3].idle_s = 100;
        let prev = t.clone();
        // Default stall_after (300s): 100s idle is not a stall.
        assert!(build(&t, &prev, &bundle.settings).is_empty());
        apply_set(
            &mut bundle.settings,
            &json!({"stall_after": 60, "cadence": 30}),
        );
        assert_eq!(bundle.settings.cadence_s, 30);
        let ds = build(&t, &prev, &bundle.settings);
        assert!(digest_for(&ds, "A").unwrap().lines[0].contains("STALLED"));
    }

    #[test]
    fn quiet_mode_sends_nothing_without_change() {
        let t = two_by_three();
        let ds = build(&t, &t, &Settings::default());
        assert!(ds.is_empty());
    }

    #[test]
    fn lead_without_ai_digest_goes_to_board() {
        let mut t = two_by_three();
        t[1].has_ai = false; // A has no AI
        let ds = build(&t, &[], &Settings::default());
        assert!(digest_for(&ds, "A").is_none());
        assert_eq!(digest_for(&ds, BOARD).unwrap().lines.len(), 3);
    }

    #[test]
    fn register_team_registers_cadencer() {
        let mut reg = fantastic_kernel::BundleRegistry::new();
        crate::register_team(&mut reg);
        let names: Vec<&str> = reg.catalog().map(|(n, _)| n).collect();
        assert!(names.contains(&HANDLER_MODULE));
    }

    #[tokio::test]
    async fn cadencer_delivers_a_group_digest_to_its_driver() {
        let mut board = crate::board::Board::default();
        board.touch_agent("w1", "", 0.0, 0);
        board.touch_agent("w2", "", 0.0, 0);
        let mut groups = BTreeMap::new();
        groups.insert(
            "g1".to_string(),
            crate::board::Group {
                driver: "d1".into(),
                members: vec!["w1".into(), "w2".into()],
            },
        );
        let quiet = BTreeMap::new();
        let mut busy = BTreeMap::new();
        busy.insert(
            "w1".to_string(),
            crate::board::Live {
                generating: true,
                cache_hit: 0.0,
                ..Default::default()
            },
        );
        let to_nodes = |rows: Vec<Value>| -> Vec<Node> {
            crate::cadence_feed::tree_from_board(&rows)
                .iter()
                .filter_map(node_from)
                .collect()
        };
        let prev = to_nodes(crate::board::rows_from(&board, &groups, &quiet, 10));
        let tree = to_nodes(crate::board::rows_from(&board, &groups, &busy, 10));
        let digests = build(&tree, &prev, &Settings::default());
        let (sent, peers) = fake(false);
        deliver(&shared(), Some(peers), digests).await;
        let got = sent.lock().unwrap().clone();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "d1");
        assert!(got[0].1.contains("w1"));
        assert!(!got[0].1.contains("w2"));
    }

    #[test]
    fn digests_follow_real_work() {
        let mut board = crate::board::Board::default();
        board.touch_agent("w1", "", 0.0, 0);
        let mut groups = BTreeMap::new();
        groups.insert(
            "g1".to_string(),
            crate::board::Group {
                driver: "d1".into(),
                members: vec!["w1".into()],
            },
        );
        let live = |inquiry: &str, newest_row: u64| {
            let mut m = BTreeMap::new();
            m.insert(
                "w1".to_string(),
                crate::board::Live {
                    inquiry: Some(inquiry.to_string()),
                    newest_row: Some(newest_row),
                    ..Default::default()
                },
            );
            m
        };
        let nodes = |lv: &BTreeMap<String, crate::board::Live>, now: u64| -> Vec<Node> {
            let rows = crate::board::rows_from(&board, &groups, lv, now);
            crate::cadence_feed::tree_from_board(&rows)
                .iter()
                .filter_map(node_from)
                .collect()
        };
        let s = Settings::default();
        let first = nodes(&live("build", 5), 10);
        // The worker's newest row did not move and its task is the same: nothing to say.
        let unmoved = nodes(&live("build", 5), 20);
        assert!(build(&unmoved, &first, &s).is_empty());
        // The worker moved on to new work: a changed line goes to the driver.
        let moved = nodes(&live("test", 18), 20);
        let ds = build(&moved, &first, &s);
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].to, "d1");
        assert!(ds[0].lines[0].contains("w1") && ds[0].lines[0].contains("test"));
    }

    // ---- dispatch, timer and status tests ----

    fn shared() -> Arc<Mutex<Inner>> {
        Arc::new(Mutex::new(Inner::default()))
    }

    fn status(b: &Arc<Mutex<Inner>>) -> Value {
        dispatch(b, &json!({"type": "status"}))
    }

    /// Advance paused time by `secs`, then one more ms so timer tasks due at
    /// the boundary have run.
    async fn run_for(secs: u64) {
        tokio::time::sleep(Duration::from_secs(secs)).await;
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    fn tick_with(a1_idle: u64, a1_task: &str) -> Value {
        json!({"type": "tick", "tree": [
            {"id": "root", "has_ai": true, "state": "busy", "task": "x", "idle_s": 0},
            {"id": "A", "parent": "root", "has_ai": true, "state": "busy", "task": "x", "idle_s": 0},
            {"id": "A1", "parent": "A", "has_ai": true, "state": "busy", "task": a1_task, "idle_s": a1_idle}
        ]})
    }

    fn lines_for(reply: &Value, to: &str) -> Vec<String> {
        reply["digests"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d["to"] == to)
            .flat_map(|d| d["lines"].as_array().cloned().unwrap_or_default())
            .map(|l| l.as_str().unwrap_or("").to_string())
            .collect()
    }

    #[tokio::test(start_paused = true)]
    async fn dispatcher_ticks_on_its_own_timer() {
        let b = shared();
        dispatch(&b, &json!({"type": "set", "sender": "user", "cadence": 60}));
        run_for(180).await;
        assert_eq!(status(&b)["ticks"], 3);
    }

    #[tokio::test(start_paused = true)]
    async fn dispatcher_off_runs_no_timer() {
        let b = shared();
        dispatch(
            &b,
            &json!({"type": "set", "sender": "user", "enabled": false}),
        );
        run_for(600).await;
        let s = status(&b);
        assert_eq!(s["ticks"], 0);
        assert!(s["next_beat_s"].is_null());
    }

    #[tokio::test(start_paused = true)]
    async fn parent_driver_sets_child_cadence_live() {
        let b = shared();
        dispatch(&b, &json!({"type": "configure", "parent": "lead"}));
        dispatch(&b, &json!({"type": "set", "sender": "user", "cadence": 60}));
        run_for(30).await;
        let before = status(&b)["ticks"].as_u64().unwrap();
        dispatch(&b, &json!({"type": "set", "sender": "lead", "cadence": 10}));
        run_for(30).await;
        assert_eq!(status(&b)["ticks"].as_u64().unwrap(), before + 3);
    }

    #[tokio::test(start_paused = true)]
    async fn set_refused_from_a_non_parent() {
        let b = shared();
        dispatch(&b, &json!({"type": "configure", "parent": "lead"}));
        dispatch(&b, &json!({"type": "set", "sender": "user", "cadence": 60}));
        {
            let g = b.lock().unwrap();
            assert!(!allowed(&g, Some("stranger")));
            assert!(allowed(&g, Some("lead")));
            assert!(allowed(&g, None));
            assert!(!allowed(&g, Some("worker_view")));
        }
        assert_eq!(status(&b)["settings"]["cadence"], 60);
    }

    #[test]
    fn stalled_worker_flagged_once_then_while_still_stalled() {
        let b = shared();
        let d1 = dispatch(&b, &tick_with(400, "x"));
        let l1 = lines_for(&d1, "A");
        assert_eq!(l1.len(), 1);
        assert!(l1[0].contains("STALLED"));
        // Still idle and growing: reported again.
        let d2 = dispatch(&b, &tick_with(460, "x"));
        let l2 = lines_for(&d2, "A");
        assert_eq!(l2.len(), 1);
        assert!(l2[0].contains("STALLED"));
        // It moves: stall gone, nothing sent for A.
        let d3 = dispatch(&b, &tick_with(0, "x"));
        assert!(lines_for(&d3, "A").is_empty());
    }

    #[test]
    fn idle_member_without_a_task_is_not_stalled_live() {
        let b = shared();
        let tree = json!({"type": "tick", "tree": [
            {"id": "L", "has_ai": true, "state": "busy", "task": "x", "idle_s": 0},
            {"id": "reviewer", "parent": "L", "has_ai": true, "state": "idle", "task": "", "idle_s": 600}
        ]});
        for _ in 0..2 {
            let d = dispatch(&b, &tree);
            assert!(lines_for(&d, "L").iter().all(|l| !l.contains("STALLED")));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn status_reports_last_and_next_beat_and_digests_sent() {
        let b = shared();
        dispatch(&b, &json!({"type": "set", "sender": "user", "cadence": 60}));
        let first = dispatch(&b, &tick_with(0, "x"));
        let n1 = first["digests"].as_array().unwrap().len() as u64;
        run_for(60).await;
        let s = status(&b);
        assert_eq!(s["ticks"], 2);
        assert_eq!(s["digests_sent"].as_u64().unwrap(), n1);
        assert_eq!(
            s["next_beat_s"].as_u64().unwrap() - s["last_beat_s"].as_u64().unwrap(),
            60
        );
    }

    struct FakePeers {
        sent: Arc<Mutex<Vec<(String, String)>>>,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl Peers for FakePeers {
        async fn ask(&self, to: &str, p: Value) -> Value {
            if self.fail {
                return json!({"error": "down"});
            }
            let text = p["text"].as_str().unwrap_or("").to_string();
            self.sent.lock().unwrap().push((to.to_string(), text));
            json!({"ok": true})
        }
    }

    type Sent = Arc<Mutex<Vec<(String, String)>>>;

    fn fake(fail: bool) -> (Sent, Arc<dyn Peers>) {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let peers: Arc<dyn Peers> = Arc::new(FakePeers {
            sent: sent.clone(),
            fail,
        });
        (sent, peers)
    }

    #[tokio::test]
    async fn a_beat_sends_each_digest_to_its_driver() {
        let (sent, peers) = fake(false);
        let tree = vec![
            n("g1", None, true, "busy", 0),
            n("w1", Some("g1"), true, "busy", 0),
        ];
        let digests = build(&tree, &[], &Settings::default());
        deliver(&shared(), Some(peers), digests).await;
        let got = sent.lock().unwrap().clone();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "g1");
        assert!(got[0].1.contains("w1"));
    }

    #[tokio::test]
    async fn a_beat_with_nothing_changed_sends_nothing() {
        let (sent, peers) = fake(false);
        let tree = vec![
            n("g1", None, true, "busy", 0),
            n("w1", Some("g1"), true, "busy", 0),
        ];
        let digests = build(&tree, &tree, &Settings::default());
        deliver(&shared(), Some(peers), digests).await;
        assert!(sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_failed_delivery_is_counted_not_fatal() {
        let (_sent, peers) = fake(true);
        let b = shared();
        let digests = vec![Digest {
            to: "g1".into(),
            lines: vec!["w1: busy".into()],
        }];
        deliver(&b, Some(peers), digests).await;
        assert_eq!(status(&b)["delivery_errors"], 1);
        let next = dispatch(&b, &tick_with(0, "x"));
        assert!(next["sent"].is_number());
        assert_eq!(status(&b)["ticks"], 1);
    }

    #[tokio::test]
    async fn status_shows_digest_bytes_per_hour() {
        let (_sent, peers) = fake(false);
        let b = shared();
        let digests = vec![
            Digest {
                to: "g1".into(),
                lines: vec!["a: x".into()],
            },
            Digest {
                to: "g2".into(),
                lines: vec!["b: yy".into()],
            },
        ];
        let want: u64 = digests
            .iter()
            .map(|d| d.lines.join("\n").len() as u64)
            .sum();
        deliver(&b, Some(peers), digests).await;
        assert_eq!(status(&b)["digest_bytes_per_hour"].as_u64().unwrap(), want);
    }

    #[tokio::test(start_paused = true)]
    async fn a_set_that_keeps_the_cadence_keeps_the_phase() {
        let b = shared();
        dispatch(&b, &json!({"type": "set", "sender": "user", "cadence": 60}));
        run_for(90).await;
        let before = status(&b)["next_beat_s"].clone();
        dispatch(
            &b,
            &json!({"type": "set", "sender": "user", "quiet": false}),
        );
        assert_eq!(status(&b)["next_beat_s"], before);
    }

    struct BoardPeers {
        sent: Arc<Mutex<Vec<(String, String)>>>,
    }

    #[async_trait::async_trait]
    impl Peers for BoardPeers {
        async fn ask(&self, to: &str, p: Value) -> Value {
            if p["type"] == "rows" {
                return json!({"rows": [
                    {"id": "g1", "parent": null, "has_ai": true, "state": "busy", "task": "x"},
                    {"id": "w1", "parent": "g1", "has_ai": true, "state": "busy", "task": "build", "idle_s": 0}
                ]});
            }
            let text = p["text"].as_str().unwrap_or("").to_string();
            self.sent.lock().unwrap().push((to.to_string(), text));
            json!({"ok": true})
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_beat_reads_the_board_when_configured() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let b = shared();
        {
            let mut g = b.lock().unwrap();
            g.peers = Some(Arc::new(BoardPeers { sent: sent.clone() }));
            g.board = Some("board".into());
        }
        dispatch(&b, &json!({"type": "set", "sender": "user", "cadence": 60}));
        run_for(60).await;
        let got = sent.lock().unwrap().clone();
        assert!(got
            .iter()
            .any(|(to, text)| to == "g1" && text.contains("w1")));
    }

    #[test]
    fn record_patch_and_load_roundtrip() {
        let mut a = Inner {
            board: Some("b".into()),
            parent: Some("lead".into()),
            ..Inner::default()
        };
        apply_set(
            &mut a.settings,
            &json!({"quiet": false, "cadence": 5, "stall_after": 7, "lead": "g", "on": false}),
        );
        let patch = record_patch(&a);
        let mut b = Inner::default();
        assert!(load_from_record(&mut b, &patch));
        assert_eq!(b.board.as_deref(), Some("b"));
        assert_eq!(b.parent.as_deref(), Some("lead"));
        assert_eq!(patch["cadencer_parent"], "lead");
        assert!(!patch.contains_key("parent"));
        assert!(!b.settings.quiet);
        assert_eq!(b.settings.cadence_s, 5);
        assert_eq!(b.settings.stall_after_s, 7);
        assert_eq!(b.settings.off, BTreeSet::from(["g".to_string()]));
        assert_eq!(b.settings, a.settings);
    }

    #[test]
    fn empty_record_loads_nothing() {
        let mut b = Inner::default();
        assert!(!load_from_record(&mut b, &serde_json::Map::new()));
        assert_eq!(b.settings, Settings::default());
        assert!(b.board.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn boot_from_record_restarts_timer() {
        let b = shared();
        let mut meta = serde_json::Map::new();
        meta.insert("enabled".into(), json!(true));
        meta.insert("cadence".into(), json!(5));
        meta.insert("board".into(), json!("board"));
        assert!(boot_from_meta(&b, &meta));
        assert!(!status(&b)["next_beat_s"].is_null());
    }

    #[tokio::test(start_paused = true)]
    async fn boot_with_no_saved_keys_starts_no_timer() {
        let b = shared();
        assert!(!boot_from_meta(&b, &serde_json::Map::new()));
        run_for(600).await;
        let s = status(&b);
        assert!(s["next_beat_s"].is_null());
        assert_eq!(s["ticks"], 0);
    }

    /// A board whose `rows` reply carries a `load` object, or none; its token
    /// counter is read from a shared cell so a test can move it between beats.
    struct LoadPeers {
        with_load: bool,
        tokens: Arc<Mutex<u64>>,
        sent: Arc<Mutex<Vec<(String, String)>>>,
    }

    #[async_trait::async_trait]
    impl Peers for LoadPeers {
        async fn ask(&self, to: &str, p: Value) -> Value {
            if p["type"] == "rows" {
                let mut reply = json!({"rows": [
                    {"id": "g1", "parent": null, "has_ai": true, "state": "busy", "task": "x"},
                    {"id": "w1", "parent": "g1", "has_ai": true, "state": "busy", "task": "build", "idle_s": 0}
                ]});
                if self.with_load {
                    reply["load"] = json!({
                        "workers": {"busy": 1, "idle": 0, "stalled": 0},
                        "tasks": {"held": 1, "in_review": 0, "done": 0, "blocked": 0, "queued": 2},
                        "done_last_hour": 0,
                        "tokens_total": *self.tokens.lock().unwrap(),
                    });
                }
                return reply;
            }
            let text = p["text"].as_str().unwrap_or("").to_string();
            self.sent.lock().unwrap().push((to.to_string(), text));
            json!({"ok": true})
        }
    }

    #[tokio::test(start_paused = true)]
    async fn status_reports_load_after_a_beat() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let tokens = Arc::new(Mutex::new(0));
        let b = shared();
        {
            let mut g = b.lock().unwrap();
            g.peers = Some(Arc::new(LoadPeers {
                with_load: true,
                tokens: tokens.clone(),
                sent: sent.clone(),
            }));
            g.board = Some("board".into());
        }
        dispatch(&b, &json!({"type": "set", "sender": "user", "cadence": 60}));
        run_for(60).await;
        let s = status(&b);
        assert_eq!(s["load"]["workers"]["busy"], 1);
        assert_eq!(s["load"]["tasks"]["queued"], 2);
        assert_eq!(s["load"]["tokens_total"], 0);
        assert!(s["load"]["driver_tokens_per_hour"].is_u64());
        assert!(s["load"]["digest_bytes_per_hour"].is_u64());
        // The digest carries the load as one plain line, numbers only.
        let line = sent
            .lock()
            .unwrap()
            .iter()
            .map(|(_, t)| t.clone())
            .find(|t| t.contains("load: "))
            .expect("a digest with the load line");
        assert!(line.contains("queued 2"), "{line}");
        assert!(!line.contains('/') || line.contains("done/h"), "{line}");
    }

    #[tokio::test(start_paused = true)]
    async fn read_board_still_works_without_load() {
        let tokens = Arc::new(Mutex::new(0));
        let peers = LoadPeers {
            with_load: false,
            tokens,
            sent: Arc::new(Mutex::new(Vec::new())),
        };
        let (nodes, load) = read_board(&peers, "board", 300).await.expect("rows read");
        assert_eq!(nodes.len(), 2);
        assert!(load.is_null());
    }

    #[tokio::test(start_paused = true)]
    async fn driver_tokens_per_hour_is_fed_by_counters() {
        let tokens = Arc::new(Mutex::new(1000));
        let b = shared();
        {
            let mut g = b.lock().unwrap();
            g.peers = Some(Arc::new(LoadPeers {
                with_load: true,
                tokens: tokens.clone(),
                sent: Arc::new(Mutex::new(Vec::new())),
            }));
            g.board = Some("board".into());
        }
        dispatch(&b, &json!({"type": "set", "sender": "user", "cadence": 60}));
        run_for(60).await; // first beat: counter 1000
        *tokens.lock().unwrap() = 1500;
        run_for(60).await; // second beat: counter 1500, so 500 more
        assert_eq!(status(&b)["driver_tokens_per_hour"], 1500);
        run_for(3600).await; // both beats are now outside the hour
        assert_eq!(status(&b)["driver_tokens_per_hour"], 0);
    }

    #[tokio::test(start_paused = true)]
    async fn driver_tokens_survive_a_counter_reset() {
        let tokens = Arc::new(Mutex::new(1000));
        let b = shared();
        {
            let mut g = b.lock().unwrap();
            g.peers = Some(Arc::new(LoadPeers {
                with_load: true,
                tokens: tokens.clone(),
                sent: Arc::new(Mutex::new(Vec::new())),
            }));
            g.board = Some("board".into());
        }
        dispatch(&b, &json!({"type": "set", "sender": "user", "cadence": 60}));
        run_for(60).await; // first beat: counter 1000, all of it new
        *tokens.lock().unwrap() = 200; // the agent restarted: its counter began again
        run_for(60).await; // second beat: 200 tokens since the restart
        assert_eq!(status(&b)["driver_tokens_per_hour"], 1200);
    }

    /// The board's load for a fixed list (`done` tasks done, `pending` queued),
    /// or for a refilling queue when `recent`: its done tasks finished in the last hour.
    fn board_load(done: usize, pending: usize, recent: bool) -> Value {
        let now = 10_000;
        let mut b = crate::board::Board::default();
        let mut put = |i: usize, status: crate::board::Status, updated: u64| {
            let id = format!("o/r#{i}");
            let task = crate::board::Task {
                id: id.clone(),
                status,
                updated,
                ..Default::default()
            };
            b.tasks.insert(id, task);
        };
        for i in 0..done {
            put(
                i,
                crate::board::Status::Done,
                if recent { now - 10 } else { now - 9_000 },
            );
        }
        for i in 0..pending {
            put(1_000 + i, crate::board::Status::Pending, now);
        }
        crate::board::load_numbers(&b, &BTreeMap::new(), &BTreeMap::new(), now, 300)
    }

    /// The group's driver model, scripted: it records each digest it is sent,
    /// answers with the reply it was given, and the board answers `rows` with a fixed load.
    struct ScriptedDriver {
        load: Value,
        reply: String,
        seen: Mutex<Vec<String>>,
        answered: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl Peers for ScriptedDriver {
        async fn ask(&self, to: &str, p: Value) -> Value {
            match p["type"].as_str().unwrap_or("") {
                "rows" => json!({
                    "rows": [
                        {"id": "g1", "parent": null, "has_ai": true, "driver": "d1", "state": "busy", "task": "x"},
                        {"id": "w1", "parent": "g1", "has_ai": true, "state": "busy", "task": "build", "idle_s": 0}
                    ],
                    "load": self.load,
                }),
                "send" if to == "d1" => {
                    self.seen
                        .lock()
                        .unwrap()
                        .push(p["text"].as_str().unwrap_or("").to_string());
                    self.answered.lock().unwrap().push(self.reply.clone());
                    json!({"ok": true, "reply": self.reply})
                }
                _ => json!({"error": "unexpected"}),
            }
        }
    }

    /// One real beat: read the board as the timer does, `beat`, then `deliver`.
    async fn beat_and_deliver(
        load: Value,
        reply: &str,
    ) -> (Arc<ScriptedDriver>, Arc<Mutex<Inner>>) {
        let driver = Arc::new(ScriptedDriver {
            load,
            reply: reply.into(),
            seen: Mutex::new(Vec::new()),
            answered: Mutex::new(Vec::new()),
        });
        let peers: Arc<dyn Peers> = driver.clone();
        let b = shared();
        b.lock().unwrap().board = Some("board".into());
        let (tree, load) = read_board(peers.as_ref(), "board", 300)
            .await
            .expect("rows read");
        let digests = {
            let mut g = b.lock().unwrap();
            g.tree = tree;
            g.load = load;
            beat(&mut g)
        };
        deliver(&b, Some(peers), digests).await;
        (driver, b)
    }

    #[tokio::test(start_paused = true)]
    async fn scripted_driver_receives_the_beat_digest_via_deliver() {
        // A fixed list: 12 done, 17 queued.
        let (fixed, fb) = beat_and_deliver(board_load(12, 17, false), "12 / 29 done").await;
        let seen = fixed.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "one digest to the driver");
        assert!(seen[0].contains("queued 17"), "{}", seen[0]);
        assert!(seen[0].contains("done 12"), "{}", seen[0]);
        assert_eq!(fixed.answered.lock().unwrap().as_slice(), ["12 / 29 done"]);
        assert_eq!(status(&fb)["delivery_errors"], 0);

        // A refilling queue: 5 queued, 12 done in the last hour, no fixed total.
        let (queue, qb) = beat_and_deliver(
            board_load(12, 5, true),
            "load: 5 queued, 12 done in the last hour",
        )
        .await;
        let seen = queue.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "one digest to the driver");
        assert!(seen[0].contains("queued 5"), "{}", seen[0]);
        assert!(seen[0].contains("done/h 12"), "{}", seen[0]);
        assert_eq!(
            queue.answered.lock().unwrap().as_slice(),
            ["load: 5 queued, 12 done in the last hour"]
        );
        assert_eq!(status(&qb)["delivery_errors"], 0);
    }

    /// A lead `A` (driver `drv`) with two members; nothing counts as changed.
    fn quiet_inner(states: [&str; 2], tasks: [&str; 2], queued: u64) -> Inner {
        let mut a = n("A", None, true, "busy", 0);
        a.driver = "drv".into();
        let mut m1 = n("A1", Some("A"), true, states[0], 0);
        m1.task = tasks[0].into();
        let mut m2 = n("A2", Some("A"), true, states[1], 0);
        m2.task = tasks[1].into();
        let mut i = Inner::default();
        i.tree = vec![a, m1, m2];
        i.prev = i.tree.clone();
        i.load = json!({"tasks": {"queued": queued}});
        i
    }

    fn to_a(ds: &[Digest]) -> Vec<&Digest> {
        ds.iter().filter(|d| d.to == "drv").collect()
    }

    #[test]
    fn quiet_mode_still_sends_a_heartbeat_when_members_are_idle() {
        let mut i = quiet_inner(["idle", "idle"], ["", ""], 4);
        let sent: Vec<usize> = (1..=6).map(|_| to_a(&beat(&mut i)).len()).collect();
        assert_eq!(sent, [0, 0, 1, 0, 0, 1]);
        // Beat 7, 8, 9: the third one carries the line.
        i.quiet_beats.clear();
        for _ in 0..2 {
            assert!(beat(&mut i).is_empty());
        }
        let ds = beat(&mut i);
        let a = to_a(&ds);
        assert_eq!(a.len(), 1);
        assert!(a[0].lines[0].starts_with("heartbeat"), "{:?}", a[0].lines);
        assert!(a[0].lines[0].contains("2 idle") && a[0].lines[0].contains("4 queued"));
    }

    #[test]
    fn heartbeat_not_sent_without_queued_work() {
        let mut i = quiet_inner(["idle", "idle"], ["", ""], 0);
        for _ in 0..6 {
            assert!(beat(&mut i).is_empty());
        }
    }

    #[test]
    fn heartbeat_not_sent_when_no_member_is_idle() {
        let mut i = quiet_inner(["busy", "busy"], ["t", "t"], 4);
        for _ in 0..6 {
            assert!(beat(&mut i).is_empty());
        }
        // An idle member that still holds a task is not idle either.
        let mut i = quiet_inner(["idle", "idle"], ["t", "t"], 4);
        for _ in 0..6 {
            assert!(beat(&mut i).is_empty());
        }
    }

    #[test]
    fn heartbeat_counter_resets_after_a_regular_digest() {
        let mut i = quiet_inner(["idle", "idle"], ["", ""], 4);
        assert!(beat(&mut i).is_empty());
        assert!(beat(&mut i).is_empty());
        // A change: the regular digest goes out, the counter restarts.
        i.tree[1].state = "busy".into();
        i.tree[1].task = "t".into();
        let ds = beat(&mut i);
        assert_eq!(to_a(&ds).len(), 1);
        assert!(!ds[0].lines[0].starts_with("heartbeat"));
        assert!(beat(&mut i).is_empty());
        assert!(beat(&mut i).is_empty());
        let ds = beat(&mut i);
        assert_eq!(to_a(&ds).len(), 1);
        assert!(ds[0].lines[0].starts_with("heartbeat"));
    }

    #[test]
    fn heartbeat_skips_a_lead_switched_off() {
        let mut i = quiet_inner(["idle", "idle"], ["", ""], 4);
        i.settings.off.insert("A".into());
        for _ in 0..6 {
            assert!(beat(&mut i).is_empty());
        }
    }

    #[test]
    fn heartbeat_is_not_sent_when_quiet_is_off() {
        let mut i = quiet_inner(["idle", "idle"], ["", ""], 4);
        i.settings.quiet = false;
        // Not quiet: every beat sends the full lines, and no line is a heartbeat.
        for _ in 0..4 {
            let ds = beat(&mut i);
            assert!(ds.iter().all(|d| !d.lines[0].starts_with("heartbeat")));
        }
    }

    #[test]
    fn a_group_override_changes_stall_after_for_that_group_only() {
        let mut t = vec![];
        for (lead, w) in [("A", "A1"), ("B", "B1")] {
            let mut l = n(lead, None, true, "busy", 0);
            l.driver = format!("drv_{lead}");
            t.push(l);
            t.push(n(w, Some(lead), true, "busy", 400));
        }
        let groups: Groups = [(
            "A".to_string(),
            GroupSettings {
                stall_after_s: Some(500),
                ..GroupSettings::default()
            },
        )]
        .into();
        let ds = build_with(&t, &t, &Settings::default(), &groups);
        // B uses the global 300 s and flags the stall; A waits for 500 s.
        assert!(ds.iter().all(|(l, _)| l != "A"));
        let b = ds.iter().find(|(l, _)| l == "B").expect("B digest");
        assert!(b.1.lines[0].contains("STALLED"));
    }

    #[test]
    fn a_group_with_enabled_false_gets_no_digest() {
        let mut t = quiet_inner(["busy", "busy"], ["t", "t"], 0).tree;
        t[1].state = "failed".into();
        let groups: Groups = [(
            "A".to_string(),
            GroupSettings {
                enabled: Some(false),
                ..GroupSettings::default()
            },
        )]
        .into();
        assert!(build_with(&t, &[], &Settings::default(), &groups).is_empty());
        assert_eq!(
            build_with(&t, &[], &Settings::default(), &Groups::new()).len(),
            1
        );
    }

    #[test]
    fn a_group_quiet_false_overrides_the_global_quiet() {
        let t = quiet_inner(["busy", "busy"], ["t", "t"], 0).tree;
        let groups: Groups = [(
            "A".to_string(),
            GroupSettings {
                quiet: Some(false),
                ..GroupSettings::default()
            },
        )]
        .into();
        assert!(build_with(&t, &t, &Settings::default(), &Groups::new()).is_empty());
        assert_eq!(build_with(&t, &t, &Settings::default(), &groups).len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_group_cadence_longer_than_the_beat_skips_beats() {
        let mut i = quiet_inner(["busy", "busy"], ["t", "t"], 0);
        i.groups.insert(
            "A".into(),
            GroupSettings {
                cadence_s: Some(180),
                ..GroupSettings::default()
            },
        );
        i.prev = Vec::new();
        assert_eq!(beat(&mut i).len(), 1, "the first digest goes out");
        // Changes during the skipped beats are kept for the due beat.
        i.tree[1].state = "failed".into();
        tokio::time::advance(Duration::from_secs(60)).await;
        assert!(beat(&mut i).is_empty(), "60 s after the last digest");
        tokio::time::advance(Duration::from_secs(60)).await;
        assert!(beat(&mut i).is_empty(), "120 s after the last digest");
        tokio::time::advance(Duration::from_secs(60)).await;
        let ds = beat(&mut i);
        assert_eq!(ds.len(), 1, "180 s: due");
        assert!(
            ds[0].lines.iter().any(|l| l.contains("failed")),
            "{:?}",
            ds[0].lines
        );
    }

    #[test]
    fn group_settings_roundtrip_through_the_record() {
        let mut a = Inner::default();
        a.groups.insert(
            "g".into(),
            GroupSettings {
                cadence_s: Some(600),
                stall_after_s: Some(900),
                quiet: Some(false),
                enabled: Some(false),
            },
        );
        a.groups.insert("h".into(), GroupSettings::default());
        let patch = record_patch(&a);
        assert_eq!(
            patch["group_settings"]["g"],
            json!({"cadence": 600, "stall_after": 900, "quiet": false, "enabled": false})
        );
        let mut b = Inner::default();
        assert!(load_from_record(&mut b, &patch));
        assert_eq!(b.groups, a.groups);
    }

    /// The live check the issue asks for: after a real beat (the board read, the
    /// digest built and delivered to the driver) `status` shows the digest bytes and
    /// the driver's tokens per hour. The fake `Peers` stands in for the driver, since
    /// no stock agent of `registry()` answers `send` and `reflect` with a usage.
    #[tokio::test(start_paused = true)]
    async fn status_shows_digest_bytes_and_driver_tokens_per_hour_after_a_live_beat() {
        let mut load = board_load(12, 17, false);
        load["tokens_total"] = json!(1234);
        let (driver, b) = beat_and_deliver(load, "ok").await;
        let seen = driver.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        let st = status(&b);
        assert_eq!(
            st["digest_bytes_per_hour"].as_u64(),
            Some(seen[0].len() as u64),
            "{st}"
        );
        assert!(st["digest_bytes_per_hour"].as_u64().unwrap() > 0, "{st}");
        assert_eq!(st["driver_tokens_per_hour"], 1234, "{st}");
        // The same figures sit in the load object the dispatcher card reads.
        assert_eq!(st["load"]["driver_tokens_per_hour"], 1234, "{st}");
        assert_eq!(
            st["load"]["digest_bytes_per_hour"], st["digest_bytes_per_hour"],
            "{st}"
        );
    }

    #[test]
    fn allowed_refuses_the_removed_dispatcher_view() {
        let g = Inner {
            parent: Some("boss".into()),
            ..Inner::default()
        };
        assert!(!allowed(&g, Some("dispatcher_view")));
        assert!(!allowed(&g, Some("stranger")));
        for who in ["maintainer", "user", "mcp", "boss"] {
            assert!(allowed(&g, Some(who)), "{who}");
        }
        assert!(allowed(&g, None));
    }

    #[tokio::test(start_paused = true)]
    async fn set_enabled_from_the_page_is_persisted() {
        let b = shared();
        let reply = dispatch(&b, &json!({"type": "set", "enabled": false}));
        assert_eq!(reply["enabled"], false);
        let patch = record_patch(&b.lock().unwrap());
        assert_eq!(patch["enabled"], false);
        let mut restored = Inner::default();
        assert!(load_from_record(&mut restored, &patch));
        assert!(!restored.settings.enabled);
        // And back on.
        dispatch(&b, &json!({"type": "set", "enabled": true}));
        let patch = record_patch(&b.lock().unwrap());
        assert!(load_from_record(&mut restored, &patch));
        assert!(restored.settings.enabled);
    }

    #[test]
    fn roles_key_loads_from_the_record() {
        let mut meta = serde_json::Map::new();
        meta.insert("roles".into(), json!("roles"));
        let mut b = Inner::default();
        assert!(load_from_record(&mut b, &meta));
        assert_eq!(b.roles.as_deref(), Some("roles"));
    }

    #[tokio::test(start_paused = true)]
    async fn timer_starts_with_roles_and_no_board() {
        let b = shared();
        let mut meta = serde_json::Map::new();
        meta.insert("enabled".into(), json!(true));
        meta.insert("roles".into(), json!("roles"));
        assert!(boot_from_meta(&b, &meta));
        assert!(b.lock().unwrap().board.is_none());
        assert!(!status(&b)["next_beat_s"].is_null());
        // Switched off at creation: no timer.
        let off = shared();
        meta.insert("enabled".into(), json!(false));
        boot_from_meta(&off, &meta);
        assert!(status(&off)["next_beat_s"].is_null());
    }

    fn wall_now() -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
    }

    /// The registries of a spawned team, scripted: project_lead `pl` with a sub-lead
    /// `sub` (child `w1`) and workers `w2`, `w3`; it records every `send`.
    struct RolesPeers {
        tasks: Value,
        asked: Mutex<Vec<(String, String)>>,
        sent: Mutex<Vec<(String, String)>>,
    }

    impl RolesPeers {
        fn new(tasks: Value) -> Arc<Self> {
            Arc::new(RolesPeers {
                tasks,
                asked: Mutex::new(Vec::new()),
                sent: Mutex::new(Vec::new()),
            })
        }
        fn sent_to(&self, to: &str) -> Vec<String> {
            self.sent
                .lock()
                .unwrap()
                .iter()
                .filter(|(t, _)| t == to)
                .map(|(_, x)| x.clone())
                .collect()
        }
    }

    #[async_trait::async_trait]
    impl Peers for RolesPeers {
        async fn ask(&self, to: &str, p: Value) -> Value {
            let t = p["type"].as_str().unwrap_or("").to_string();
            self.asked.lock().unwrap().push((to.to_string(), t.clone()));
            match (to, t.as_str()) {
                ("roles", "fleet") => json!({"roles": {
                    "project_lead": [{"id": "pl"}], "lead": [{"id": "sub"}],
                    "worker": [{"id": "w1"}, {"id": "w2"}, {"id": "w3"}]}}),
                ("roles", "get_count") => json!({"running": 5, "max": 8}),
                ("kernel", "reflect") => json!({"tree": [{"id": "pl", "children": [
                    {"id": "sub", "children": [{"id": "w1", "children": []}]},
                    {"id": "w2", "children": []},
                    {"id": "w3", "children": []}]}]}),
                ("tasks", "list") => self.tasks.clone(),
                (_, "send") => {
                    self.sent
                        .lock()
                        .unwrap()
                        .push((to.to_string(), p["text"].as_str().unwrap_or("").to_string()));
                    json!({"ok": true})
                }
                _ => json!({"error": "unexpected"}),
            }
        }
    }

    fn roles_inner(peers: Arc<RolesPeers>) -> Arc<Mutex<Inner>> {
        let b = shared();
        {
            let mut g = b.lock().unwrap();
            g.peers = Some(peers);
            g.roles = Some("roles".into());
        }
        b
    }

    fn running(id: &str, holder: &str, age_s: f64) -> Value {
        json!({"id": id, "source": "s", "holder": holder, "status": "running",
            "history": [{"from": null, "to": holder, "by": "pl", "at": wall_now() - age_s}], "report": null})
    }

    #[tokio::test]
    async fn roles_beat_digests_the_project_lead_and_sub_leads() {
        let peers =
            RolesPeers::new(json!({"tasks": [running("t1", "w1", 5.0), running("t2", "w2", 5.0)]}));
        let b = roles_inner(peers.clone());
        beat_once(&b).await;
        let pl = peers.sent_to("pl").join("\n");
        assert!(
            pl.contains("sub:") && pl.contains("w2:") && pl.contains("w3:"),
            "{pl}"
        );
        assert!(!pl.contains("w1:"), "the project_lead sees one level: {pl}");
        let sub = peers.sent_to("sub").join("\n");
        assert!(sub.contains("w1:") && !sub.contains("w2:"), "{sub}");
        assert!(peers.sent_to("w2").is_empty(), "a worker has no children");
        assert!(peers.asked.lock().unwrap().iter().all(|(_, t)| t != "rows"));
        assert_eq!(status(&b)["delivery_errors"], 0);
    }

    #[tokio::test]
    async fn stalled_spawned_agent_is_flagged_in_its_leads_digest() {
        let peers = RolesPeers::new(
            json!({"tasks": [running("t1", "w1", 1000.0), running("t2", "w2", 5.0)]}),
        );
        let b = roles_inner(peers.clone());
        beat_once(&b).await;
        let sub = peers.sent_to("sub").join("\n");
        assert!(sub.contains("w1:") && sub.contains("STALLED"), "{sub}");
        let pl = peers.sent_to("pl").join("\n");
        let w2 = pl.lines().find(|l| l.starts_with("w2:")).expect("w2 line");
        assert!(!w2.contains("STALLED"), "{w2}");
        let sub_line = pl
            .lines()
            .find(|l| l.starts_with("sub:"))
            .expect("sub line");
        assert!(sub_line.contains("1 STALLED"), "{sub_line}");
    }

    #[tokio::test]
    async fn roles_heartbeat_when_idle_and_queued() {
        // `sub` and `w3` hold nothing (two idle children of `pl`); a waiting task with
        // no holder is queued work.
        let waiting =
            json!({"id": "q1", "source": "s", "holder": null, "status": "waiting", "history": []});
        let peers = RolesPeers::new(
            json!({"tasks": [running("t1", "w1", 5.0), running("t2", "w2", 5.0), waiting]}),
        );
        let b = roles_inner(peers.clone());
        beat_once(&b).await;
        let first = peers.sent_to("pl");
        assert!(first.iter().all(|t| !t.contains("heartbeat")), "{first:?}");
        for _ in 0..HEARTBEAT_EVERY {
            beat_once(&b).await;
        }
        let all = peers.sent_to("pl").join("\n");
        assert!(all.contains("heartbeat: 2 idle, 1 queued"), "{all}");
    }

    #[tokio::test]
    async fn team_without_roles_keeps_board_rows() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let peers = BoardPeers { sent };
        let (nodes, _) = read_source(&peers, Some("board"), None, 300)
            .await
            .expect("the board path");
        assert!(nodes
            .iter()
            .any(|n| n.id == "w1" && n.parent.as_deref() == Some("g1")));
        assert!(read_source(&peers, None, None, 300).await.is_none());
        // With roles the spawned tree is read, and the board is not asked.
        let rp = RolesPeers::new(json!({"tasks": []}));
        let (nodes, _) = read_source(rp.as_ref(), Some("board"), Some("roles"), 300)
            .await
            .expect("the roles path");
        assert!(nodes
            .iter()
            .any(|n| n.id == "sub" && n.parent.as_deref() == Some("pl")));
        assert!(rp.asked.lock().unwrap().iter().all(|(to, _)| to != "board"));
    }
}
