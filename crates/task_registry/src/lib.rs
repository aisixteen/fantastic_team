//! `task_registry` — which agent or subtree holds which task, and where it came from.
//!
//! A generic extension between agents and task sources (gh, jira, ...): it makes NO
//! outbound call. `source` and `update_to` are opaque strings; the LLM reads them and
//! pushes updates itself. An instance whose record has no `task_store_id` refuses
//! every verb `disabled`.
//!
//! STORAGE IS ABSTRACT. The registry keeps its tables in the store agent named by its
//! record's `task_store_id`, through the store interface the agent memory uses
//! (`append {rows}`, `read {kinds, match, order, limit, after}`): the creator wires
//! `can_call` to the store and the store's `callable_by` to the registry. Each write
//! appends one snapshot row `{kind:"task", task_id, content:<entry>}`; every read takes
//! the latest row per `task_id`. Nothing is held in memory (a restart rebuilds from the
//! store) and nothing falls back to RAM: a store that refuses or is unreachable is
//! `store_refused`. A file, a path or a database is the store's business, never this
//! bundle's.
//!
//! WRITERS are branch leads. The authority is the kernel call gate: only a lead holds
//! the write verbs in its `can_call` on the registry. A worker only `send`s its
//! lead a message (agentic_resources' spawn opens that edge) and the lead writes here. The one rule inside the bundle
//! is that a `report`'s `from` is the caller or one of its descendants.
//!
//! RECORDED FACTS. An entry may also carry `versions` (`[{repo, branch, commit}]`),
//! `source_version` (an opaque string) and `fork_from` (`{agent, mark}`), set by
//! `add`/`assign`/`reassign`/`status` and kept in the reassign history, and `usage`
//! (the last `report`'s token counts). All are opaque facts: nothing is checked
//! against a repo or an agent. An entry stored before these existed loads with the
//! defaults (`[]`, null, null, null).
//!
//! Verbs: `reflect`, `add`, `assign`, `reassign`,
//! `status`, `report`, `list`, `get`.

#![deny(missing_docs)]

pub mod record;
pub mod verbs;

use async_trait::async_trait;
use fantastic_bundle::py_repr;
use verbs::{ForkFrom, Holder, State, Usage, Version};
use fantastic_kernel::bundle::{BundleError, Handler, Reply};
use fantastic_kernel::refusal::{self, missing};
use fantastic_kernel::{current_origin, Agent, AgentId, Item, Kernel};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// `handler_module` key under which this bundle registers.
pub const HANDLER_MODULE: &str = "task_registry.tools";

/// What the bundle is for: its one short man text; how to call it is rendered from its
/// declared verbs (`docs`, `reflect`).
pub const MAN: &str = "A registry of tasks: who holds each one and where it came from. It makes no outbound call; it keeps its tables in the store agent its record names.";

/// How many rows one store read asks for.
const PAGE: u64 = 500;

/// The task_registry bundle. It holds no tasks: only one lock per registry agent, so
/// the read-modify-append of two leads cannot race.
#[derive(Default)]
pub struct TaskRegistryBundle {
    locks: std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

// ── helpers ─────────────────────────────────────────────────────────

fn meta_of(a: &Agent) -> Map<String, Value> {
    a.meta.read().expect("meta poisoned").clone()
}

fn agent_of(kernel: &Kernel, id: &AgentId) -> Option<Arc<Agent>> {
    kernel.agents.get(id).map(|e| Arc::clone(&e))
}

/// The three recorded facts of a write verb: (versions, source_version, fork_from).
type Facts = (Option<Value>, Option<String>, Option<Value>);

fn facts_of(
    versions: Option<Vec<Version>>,
    source_version: Option<String>,
    fork_from: Option<ForkFrom>,
) -> Facts {
    (
        versions.map(|v| json!(v)),
        source_version,
        fork_from.map(|f| json!(f)),
    )
}

/// A `usage`, stored with all four keys ([`fantastic_ai_core::USAGE_KEYS`]).
fn usage_value(u: &Usage) -> Value {
    json!({
        "input_tokens": u.input_tokens,
        "cache_read_tokens": u.cache_read_tokens,
        "cache_write_tokens": u.cache_write_tokens,
        "output_tokens": u.output_tokens,
    })
}

/// A holder must name an existing agent (for a subtree, its root). Returns the holder as stored.
fn holder_arg(kernel: &Kernel, verb: &str, h: &Holder) -> Result<Value, Value> {
    let id = match h {
        Holder::Agent(s) | Holder::Subtree { subtree: s } => s.as_str(),
    };
    if agent_of(kernel, &AgentId::from(id)).is_none() {
        return Err(refusal::refusal(
            refusal::NOT_FOUND,
            format!("{verb}: holder {} is not an agent", py_repr(id)),
        ));
    }
    Ok(json!(h))
}

fn not_found_task(verb: &str, id: &str) -> Value {
    refusal::refusal(
        refusal::NOT_FOUND,
        format!("{verb}: task {} not found", py_repr(id)),
    )
}

// ── the store ───────────────────────────────────────────────────────

/// The store agent's id, or `None` when the registry is not configured.
fn store_id(meta: &Map<String, Value>) -> Option<String> {
    meta.get("task_store_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// ONE gated send to the store, as the registry. A refusal or a failed dispatch is
/// `store_refused`, with the store's own error and reason.
async fn store_call(kernel: &Arc<Kernel>, store: &str, payload: Value) -> Result<Value, Value> {
    let out = kernel
        .send(vec![Item::new(AgentId::from(store), payload)])
        .await
        .remove(0);
    let res = out.result;
    let error = res
        .get("error")
        .filter(|e| !e.is_null() && *e != &json!(false) && *e != &json!(""));
    if error.is_some() || out.status.as_str() != "completed" {
        let text = match error {
            Some(Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
            None => out.status.as_str().to_string(),
        };
        let mut r = refusal::refusal(
            refusal::STORE_REFUSED,
            format!("store {} refused: {text}", py_repr(store)),
        );
        refusal::with(&mut r, "store_error", json!(text));
        refusal::with(
            &mut r,
            "store_reason",
            res.get("reason").cloned().unwrap_or(Value::Null),
        );
        return Err(r);
    }
    Ok(res)
}

/// Every task, latest snapshot per `task_id`, read page by page.
async fn load(
    kernel: &Arc<Kernel>,
    store: &str,
    only: Option<&str>,
) -> Result<BTreeMap<String, Value>, Value> {
    let mut tasks = BTreeMap::new();
    let mut after = Value::Null;
    loop {
        let mut q = json!({"type": "read", "kinds": ["task"], "order": "asc", "limit": PAGE});
        if let Some(t) = only {
            q["match"] = json!({"task_id": t});
        }
        if !after.is_null() {
            q["after"] = after.clone();
        }
        let res = store_call(kernel, store, q).await?;
        for row in res
            .get("rows")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let (Some(id), Some(content)) = (
                row.get("task_id").and_then(Value::as_str),
                row.get("content").filter(|c| c.is_object()),
            ) {
                tasks.insert(id.to_string(), content.clone());
            }
        }
        match res.get("next") {
            Some(n) if !n.is_null() => after = n.clone(),
            _ => return Ok(tasks),
        }
    }
}

/// Append the new snapshot of a task.
async fn save(kernel: &Arc<Kernel>, store: &str, entry: &Value) -> Result<(), Value> {
    let row = json!({"kind": "task", "task_id": entry["id"], "content": entry});
    store_call(kernel, store, json!({"type": "append", "rows": [row]})).await?;
    Ok(())
}

/// One history step; it carries the entry's recorded facts only when it has any.
fn history_item(from: &Value, to: &Value, by: &Option<AgentId>, entry: &Value) -> Value {
    let mut item = json!({"from": from, "to": to, "by": by.as_ref().map(AgentId::as_str), "at": fantastic_ai_core::helpers::now_secs()});
    for k in ["versions", "source_version", "fork_from"] {
        let v = &entry[k];
        let empty = v.is_null() || v.as_array().is_some_and(Vec::is_empty);
        if !empty {
            item[k] = v.clone();
        }
    }
    item
}

/// Set the given facts on `entry`.
fn set_facts(entry: &mut Value, facts: Facts) {
    let (versions, source_version, fork_from) = facts;
    if let Some(v) = versions {
        entry["versions"] = v;
    }
    if let Some(s) = source_version {
        entry["source_version"] = json!(s);
    }
    if let Some(f) = fork_from {
        entry["fork_from"] = f;
    }
}

// ── the bundle ──────────────────────────────────────────────────────

impl TaskRegistryBundle {
    fn lock_of(&self, reg: &AgentId) -> Arc<tokio::sync::Mutex<()>> {
        let mut m = self.locks.lock().expect("locks poisoned");
        Arc::clone(m.entry(reg.as_str().to_string()).or_default())
    }
}

#[async_trait]
impl Handler for TaskRegistryBundle {
    type Call = verbs::Call;

    fn name(&self) -> &str {
        "task_registry"
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
        agent_id: &AgentId,
        call: Self::Call,
        kernel: &Arc<Kernel>,
    ) -> Result<Reply, BundleError> {
        let Some(me) = agent_of(kernel, agent_id) else {
            return Ok(None);
        };
        let meta = meta_of(&me);
        let store = store_id(&meta);
        if let verbs::Call::Reflect(_) = call {
            return Ok(Some(json!({
                "id": agent_id.as_str(),
                "sentence": MAN,
                "enabled": store.is_some(),
            })));
        }
        let Some(store) = store else {
            return Ok(Some(refusal::refusal_hint(
                refusal::DISABLED,
                "task_registry is disabled: the record has no task_store_id".to_string(),
                "set task_store_id on the record",
            )));
        };
        let reply = match call {
            verbs::Call::Reflect(_) => Value::Null,
            verbs::Call::List(a) => flat(list_inner(kernel, &store, a).await),
            verbs::Call::Get(a) => flat(get_inner(kernel, &store, &a.id).await),
            write => {
                let lock = self.lock_of(agent_id);
                let _held = lock.lock().await;
                let caller = current_origin();
                match write {
                    verbs::Call::Add(a) => flat(add_inner(kernel, &store, &caller, a).await),
                    verbs::Call::Assign(a) => {
                        let f = facts_of(a.versions, a.source_version, a.fork_from);
                        flat(assign_inner(kernel, &store, &caller, &a.id, a.holder, f, false).await)
                    }
                    verbs::Call::Reassign(a) => {
                        let f = facts_of(a.versions, a.source_version, a.fork_from);
                        flat(assign_inner(kernel, &store, &caller, &a.id, a.holder, f, true).await)
                    }
                    verbs::Call::Status(a) => flat(status_inner(kernel, &store, a).await),
                    verbs::Call::Report(a) => flat(report_inner(kernel, &store, &caller, a).await),
                    _ => Value::Null,
                }
            }
        };
        Ok(Some(reply))
    }
}

// ── verbs ───────────────────────────────────────────────────────────

/// Run `f` and flatten `Result<Value, Value>`.
fn flat(r: Result<Value, Value>) -> Value {
    r.unwrap_or_else(|e| e)
}

async fn add_inner(
    kernel: &Arc<Kernel>,
    store: &str,
    caller: &Option<AgentId>,
    a: verbs::Add,
) -> Result<Value, Value> {
    let (id, source) = (a.id.as_str(), a.source.as_str());
    let parent = a.parent.as_deref();
    let update_to = a.update_to.as_deref().unwrap_or(source);
    let status = a.status.as_str();
    let facts = facts_of(a.versions, a.source_version, a.fork_from);
    let holder = match &a.holder {
        None => Value::Null,
        Some(h) => holder_arg(kernel, "add", h)?,
    };
    let tasks = load(kernel, store, None).await?;
    if tasks.contains_key(id) {
        return Err(refusal::refusal(
            refusal::EXISTS,
            format!("add: task {} already exists", py_repr(id)),
        ));
    }
    if let Some(p) = parent {
        if !tasks.contains_key(p) {
            return Err(not_found_task("add", p));
        }
    }
    let mut entry = json!({
        "id": id, "parent": parent, "source": source, "update_to": update_to,
        "holder": holder, "history": [], "status": status, "report": null,
        "versions": [], "source_version": null, "fork_from": null, "usage": null,
    });
    set_facts(&mut entry, facts);
    if !holder.is_null() {
        let item = history_item(&Value::Null, &holder, caller, &entry);
        entry["history"] = json!([item]);
    }
    save(kernel, store, &entry).await?;
    Ok(entry)
}

async fn assign_inner(
    kernel: &Arc<Kernel>,
    store: &str,
    caller: &Option<AgentId>,
    id: &str,
    holder: Holder,
    facts: Facts,
    re: bool,
) -> Result<Value, Value> {
    let verb = if re { "reassign" } else { "assign" };
    let to = holder_arg(kernel, verb, &holder)?;
    let mut entry = load(kernel, store, Some(id))
        .await?
        .remove(id)
        .ok_or_else(|| not_found_task(verb, id))?;
    let from = entry["holder"].clone();
    match (re, from.is_null()) {
        (false, false) => {
            return Err(refusal::refusal(
                refusal::ALREADY_HELD,
                format!(
                    "assign: task {} is held by {from}; use reassign",
                    py_repr(id)
                ),
            ))
        }
        (true, true) => {
            return Err(refusal::refusal(
                refusal::NOT_HELD,
                format!("reassign: task {} has no holder; use assign", py_repr(id)),
            ))
        }
        _ => {}
    }
    set_facts(&mut entry, facts);
    let item = history_item(&from, &to, caller, &entry);
    entry["history"]
        .as_array_mut()
        .expect("history is a list")
        .push(item);
    entry["holder"] = to;
    save(kernel, store, &entry).await?;
    Ok(entry)
}

async fn status_inner(
    kernel: &Arc<Kernel>,
    store: &str,
    a: verbs::Status,
) -> Result<Value, Value> {
    let id = a.id.as_str();
    let status = a.status.map(State::as_str);
    let facts = facts_of(a.versions, a.source_version, a.fork_from);
    // at least one of the state and the three facts
    if status.is_none() && facts.0.is_none() && facts.1.is_none() && facts.2.is_none() {
        return Err(missing("status", "status"));
    }
    let mut entry = load(kernel, store, Some(id))
        .await?
        .remove(id)
        .ok_or_else(|| not_found_task("status", id))?;
    if let Some(s) = status {
        entry["status"] = json!(s);
    }
    set_facts(&mut entry, facts);
    save(kernel, store, &entry).await?;
    Ok(entry)
}

async fn report_inner(
    kernel: &Arc<Kernel>,
    store: &str,
    caller: &Option<AgentId>,
    a: verbs::Report,
) -> Result<Value, Value> {
    let id = a.id.as_str();
    let state = a.state.map(State::as_str);
    let result = a.result.unwrap_or(Value::Null);
    let usage = a.usage.as_ref().map(usage_value);
    let from = match a.from.as_deref() {
        Some(f) => Some(AgentId::from(f)),
        None => caller.clone(),
    };
    // the reporter is the caller or below it; the operator may report for anyone
    if let Some(who) = from.as_ref() {
        if agent_of(kernel, who).is_none() {
            return Err(refusal::refusal(
                refusal::NOT_FOUND,
                format!("report: {} is not an agent", py_repr(who.as_str())),
            ));
        }
        if let Some(c) = caller.as_ref() {
            if who != c
                && !kernel
                    .ancestor_ids(agent_of(kernel, who).as_deref())
                    .contains(c.as_str())
            {
                return Err(refusal::refusal(
                    refusal::NOT_IN_SUBTREE,
                    format!(
                        "report: {} is not {c} or one of its descendants",
                        py_repr(who.as_str())
                    ),
                ));
            }
        }
    }
    let mut entry = load(kernel, store, Some(id))
        .await?
        .remove(id)
        .ok_or_else(|| not_found_task("report", id))?;
    entry["report"] = json!({
        "from": from.as_ref().map(AgentId::as_str), "state": state,
        "result": result, "at": fantastic_ai_core::helpers::now_secs(), "usage": usage,
    });
    if usage.is_some() {
        entry["usage"] = entry["report"]["usage"].clone();
    }
    save(kernel, store, &entry).await?;
    Ok(entry)
}

async fn list_inner(
    kernel: &Arc<Kernel>,
    store: &str,
    a: verbs::List,
) -> Result<Value, Value> {
    let parent = a.parent.as_deref();
    let status = a.status.map(State::as_str);
    let holder = a.holder.as_ref().map(|h| json!(h));
    let holder = holder.as_ref();
    let ids: Option<Vec<&str>> = a.ids.as_ref().map(|v| v.iter().map(String::as_str).collect());
    let tasks = load(kernel, store, None).await?;
    if let Some(ids) = &ids {
        if let Some(unknown) = ids.iter().find(|i| !tasks.contains_key(**i)) {
            return Err(not_found_task("list", unknown));
        }
    }
    let out: Vec<Value> = tasks
        .into_values()
        .filter(|t| {
            ids.as_ref()
                .is_none_or(|ids| ids.iter().any(|i| t["id"] == json!(i)))
        })
        .filter(|t| parent.is_none_or(|p| t["parent"] == json!(p)))
        .filter(|t| status.is_none_or(|s| t["status"] == json!(s)))
        .filter(|t| holder.is_none_or(|h| &t["holder"] == h))
        .collect();
    let mut sum = Map::new();
    for k in fantastic_ai_core::USAGE_KEYS {
        let n: u64 = out
            .iter()
            .map(|t| t["usage"].get(k).and_then(Value::as_u64).unwrap_or(0))
            .sum();
        sum.insert(k.to_string(), json!(n));
    }
    Ok(json!({"tasks": out, "usage": sum}))
}

async fn get_inner(kernel: &Arc<Kernel>, store: &str, id: &str) -> Result<Value, Value> {
    let tasks = load(kernel, store, None).await?;
    let mut entry = tasks
        .get(id)
        .cloned()
        .ok_or_else(|| not_found_task("get", id))?;
    let children: Vec<&str> = tasks
        .values()
        .filter(|t| t["parent"] == json!(id))
        .filter_map(|t| t["id"].as_str())
        .collect();
    entry["children"] = json!(children);
    Ok(entry)
}

#[cfg(test)]
mod tests;
