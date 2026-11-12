//! `agentic_resources` — a generic registry of spawnable BUNDLES.
//!
//! A spawnable bundle is a META BUNDLE: a fantastic `bundles/<name>/bundle.yaml` with a
//! `base`, its directive (`record.post_system_prompt`) and its grants
//! (`record.can_call`), and no model. The registry lists them (`bundles`, `bundle`),
//! creates an agent from one (`spawn`) and reports the agents that run on them (`fleet`,
//! `get_count`).
//!
//! The child runs on its SPAWNER's backend (its handler and provider fields), with the
//! `model` the spawner names in `spawn` (its own when it names none): the spawner says
//! which model, from its own job prompt.
//!
//! NOTHING is counted or marked. An agent is its name and its bundle (`handler_module`),
//! so `fleet` and `get_count` read the tree: every agent whose bundle is a spawnable
//! meta bundle. A delete lowers the count and a restart rebuilds it. There are no
//! limits. The task tree is not kept here: that is `task_registry`.
//!
//! A spawned agent tells its parent with plain `send`: `spawn` opens the edge
//! child -> parent (`send`, both ends), so the message starts the parent's next turn.
//!
//! NO CLEANUP: nothing is retired or deleted when a child is done;
//! `delete_agent` is the only way out.
//!
//! `spawn` may branch `from` a fork point of an existing agent (`from:{agent, at}`):
//! the registry forks it (ai_core `fork {at}`). The result is a CLONE: the source's own
//! bundle (any other is refused), its record untouched, beside the source. The caller gets
//! control of it and the registry may send it its task; nothing else is written.
//!
//! Verbs: `reflect`, `bundles`, `bundle`, `spawn`, `fleet`, `get_count`.

#![deny(missing_docs)]

pub mod record;
pub mod cards;
pub mod verbs;

use async_trait::async_trait;
use fantastic_bundle::py_repr;
use fantastic_kernel::bundle::{BundleError, Handler, Reply};
use fantastic_kernel::grants::union;
use fantastic_kernel::refusal;
use fantastic_kernel::{current_origin, Agent, AgentId, Item, Kernel};
use cards::{Card, REGISTRY_TOKEN};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

/// `handler_module` key under which this bundle registers.
pub const HANDLER_MODULE: &str = "agentic_resources.tools";

/// What the bundle is for: its one short man text; how to call it is rendered from its
/// declared verbs (`docs`, `reflect`).
pub const MAN: &str = "A registry of spawnable bundles (meta bundles): list them, spawn one as a child of the caller and report the agents running on them. A spawnable bundle is a meta bundle whose record carries a directive and its grants; a child runs on its spawner's provider, and the caller controls it.";

/// What a parent holds over every agent it spawns.
const PARENT_VERBS: [&str; 3] = ["pause", "resume", "stop"];

/// What every spawned agent holds on its parent: it tells it with a message.
const CHILD_VERBS: [&str; 1] = ["send"];

/// The agentic_resources bundle.
pub struct AgenticResourcesBundle;

// ── record helpers ──────────────────────────────────────────────────

fn meta_of(a: &Agent) -> Map<String, Value> {
    a.meta.read().expect("meta poisoned").clone()
}

fn agent_of(kernel: &Kernel, id: &AgentId) -> Option<Arc<Agent>> {
    kernel.agents.get(id).map(|e| Arc::clone(&e))
}

/// One agent running on a spawnable bundle.
struct Spawned {
    id: AgentId,
    bundle: String,
}

/// Every agent whose bundle (`handler_module`) is one of the kernel's spawnable meta
/// bundles, by id.
fn spawned(kernel: &Kernel) -> Vec<Spawned> {
    let names: Vec<String> = cards::of(kernel).into_iter().map(|c| c.name).collect();
    let mut out: Vec<Spawned> = kernel
        .agents
        .iter()
        .filter_map(|e| {
            let bundle = e.value().handler_module.clone()?;
            names.contains(&bundle).then(|| Spawned {
                id: e.value().id.clone(),
                bundle,
            })
        })
        .collect();
    out.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    out
}

/// The spawnable bundles, and the bundles that failed to load (their errors).
fn cards_of(kernel: &Kernel) -> (Vec<Card>, Vec<String>) {
    let errors = kernel
        .bundles
        .folder_rows()
        .iter()
        .filter_map(|r| r.get("error").and_then(Value::as_str).map(str::to_string))
        .collect();
    (cards::of(kernel), errors)
}

fn bundles_list(cards: &[Card]) -> Value {
    Value::Array(
        cards
            .iter()
            .map(|c| json!({"name": c.name, "purpose": c.purpose}))
            .collect(),
    )
}

// ── the bundle ──────────────────────────────────────────────────────

#[async_trait]
impl Handler for AgenticResourcesBundle {
    type Call = verbs::Call;

    fn name(&self) -> &str {
        "agentic_resources"
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
        let _ = me;
        let reply = match call {
            verbs::Call::Reflect(_) => {
                let names: Vec<String> = cards_of(kernel).0.into_iter().map(|c| c.name).collect();
                json!({
                    "id": agent_id.as_str(),
                    "sentence": MAN,
                    "enabled": true, "bundles": names,
                })
            }
            verbs::Call::Bundles(_) => {
                let (cards, errors) = cards_of(kernel);
                json!({"bundles": bundles_list(&cards), "errors": errors})
            }
            verbs::Call::Bundle(a) => bundle_reply(kernel, &a.name),
            verbs::Call::Spawn(a) => spawn_reply(agent_id, a, kernel).await,
            verbs::Call::Fleet(_) => fleet_reply(kernel),
            verbs::Call::GetCount(a) => get_count_reply(kernel, a.bundle.as_deref()),
        };
        Ok(Some(reply))
    }
}

// ── verbs ───────────────────────────────────────────────────────────

fn bundle_reply(kernel: &Kernel, name: &str) -> Value {
    let (cards, _) = cards_of(kernel);
    match cards.iter().find(|c| c.name == name) {
        Some(c) => c.to_value(),
        None => refusal::refusal(
            refusal::NOT_FOUND,
            format!("bundle {} not found", py_repr(name)),
        ),
    }
}

async fn spawn_reply(
    reg: &AgentId,
    a: verbs::Spawn,
    kernel: &Arc<Kernel>,
) -> Value {
    let bundle = a.bundle.as_str();
    let (cards, _) = cards_of(kernel);
    let Some(card) = cards.iter().find(|c| c.name == bundle) else {
        return refusal::refusal(
            refusal::NOT_FOUND,
            format!("bundle {} not found", py_repr(bundle)),
        );
    };
    // the chat: on the spawner's provider (or the one named), with the model named (or the spawner's)
    let tier_fields = match chat_of(kernel, &current_origin(), &a) {
        Ok(b) => b,
        Err(r) => return r,
    };
    // the task: a string is the child's first message
    let first_message: Option<String> = a.task.clone();
    let name = a.name.clone();
    // `from`: branch the new agent off a fork point of an existing one
    let from: Option<(String, i64)> = a.from.as_ref().map(|f| (f.agent.clone(), f.at));

    let caller = current_origin();
    if let Some(refusal) = escalation(kernel, &caller, card) {
        return refusal;
    }
    let parent = caller.clone().unwrap_or_else(|| reg.clone());

    if let Some((src, at)) = from {
        let ctx = SpawnFrom {
            reg,
            card,
            caller: &caller,
            first_message,
        };
        return spawn_from(kernel, &ctx, &src, at).await;
    }

    // the child's grants: the card's, with `$registry` made real
    let mut can_call = card_can_call(reg, card);
    // the child completes through its provider
    if let Some(p) = provider_of(&tier_fields) {
        union(
            &mut can_call,
            &Map::from_iter([(String::from(&p), json!(PROVIDER_VERBS))]),
        );
    }
    // a tier that names the store the agent keeps its conversation in: the agent may
    // use it (the other end is opened below, before the first message)
    let store = tier_fields
        .get("memory_store_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if let Some(s) = &store {
        union(
            &mut can_call,
            &Map::from_iter([(String::from(s), json!("*"))]),
        );
    }
    let reg_verbs = reg_verbs_of(&can_call, reg);
    let mut callable_by = Map::new();
    union(&mut callable_by, &card.callable_by);
    if let Some(c) = &caller {
        union(
            &mut callable_by,
            &Map::from_iter([(String::from(c.as_str()), json!(PARENT_VERBS))]),
        );
    }
    if first_message.is_some() {
        union(
            &mut callable_by,
            &Map::from_iter([(String::from(reg.as_str()), json!(["send"]))]),
        );
    }

    // the agent is created as an instance of the bundle, whose fields (its directive
    // among them) lie under these
    let mut fields = Map::new();
    for (k, v) in &tier_fields {
        fields.insert(k.clone(), v.clone());
    }
    fields.insert("can_call".to_string(), Value::Object(can_call));
    fields.insert("callable_by".to_string(), Value::Object(callable_by));
    if let Some(n) = &name {
        fields.insert("display_name".to_string(), json!(n));
    }
    let Some(root) = kernel.root() else {
        return refusal::refusal(refusal::NO_ROOT, "the kernel has no root".to_string());
    };
    let made = fantastic_kernel::lifecycle::create_agent(
        kernel,
        &root,
        fantastic_kernel::declared::kernel::CreateAgent {
            handler_module: bundle.to_string(),
            parent_id: parent.as_str().to_string(),
            id: None,
            fields,
        },
    )
    .await;
    let Some(child) = made.get("id").and_then(Value::as_str).map(str::to_string) else {
        return made;
    };
    // the registry hears what the card grants on it; the parent controls its subtree
    let store_id = store
        .clone()
        .filter(|s| kernel.agents.contains_key(&AgentId::from(s.as_str())));
    if let Some(s) = store_id.clone() {
        let cid = child.clone();
        let _ = kernel.edit_callable_by(&AgentId::from(s.as_str()), move |m| {
            union(m, &Map::from_iter([(String::from(&cid), json!("*"))]));
            true
        });
    }
    let opened = open_edges(kernel, reg, caller.as_ref(), &child, reg_verbs.as_ref())
        .and_then(|()| open_tool_edges(kernel, &child, card));
    if let Err(e) = opened {
        undo_spawn(
            kernel,
            reg,
            caller.as_ref(),
            store_id.as_deref(),
            card,
            &child,
        )
        .await;
        return e;
    }
    if let Some(p) = provider_of(&tier_fields) {
        open_provider(kernel, &p, &child);
    }
    if let Some(text) = first_message {
        send_first(kernel, reg, &child, text);
    }
    json!({"id": child, "bundle": bundle, "provider": provider_of(&tier_fields),
           "model": tier_fields.get("model").cloned().unwrap_or(Value::Null)})
}

/// The chat fields a child takes from its spawner: where its conversation lives and how it
/// is kept. Never a provider's (model kind, endpoint, key, prices): those are its provider's.
const CHAT_FIELDS: [&str; 4] = [
    "memory_store_id",
    "context_window",
    "context_strategy",
    "caching",
];

/// The verbs a chat calls on its provider.
const PROVIDER_VERBS: [&str; 2] = ["complete", "watch"];

/// The child's chat record: an AI chat (its bundle's base, `ai_chat.tools`) that
/// completes through `provider_id` on `model`. `provider` and `model` in the spawn name
/// them; absent, the spawner's own (an AI spawner is a chat with a `provider_id`); no
/// `model` anywhere, the provider's default. A caller that is no chat (the operator, a
/// tool) names the `provider`.
fn chat_of(
    kernel: &Kernel,
    caller: &Option<AgentId>,
    a: &verbs::Spawn,
) -> Result<Map<String, Value>, Value> {
    let mut out = Map::new();
    let spawner = caller
        .as_ref()
        .and_then(|c| agent_of(kernel, c))
        .map(|a| meta_of(&a));
    if let Some(meta) = &spawner {
        for k in CHAT_FIELDS {
            if let Some(v) = meta.get(k) {
                out.insert(k.to_string(), v.clone());
            }
        }
    }
    let own = |k: &str| {
        spawner
            .as_ref()
            .and_then(|m| m.get(k))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let Some(provider) = a
        .provider
        .clone()
        .or_else(|| own(fantastic_ai_core::remote_provider::PROVIDER_FIELD))
    else {
        return Err(refusal::missing(
            "spawn",
            "provider (the provider agent the child completes through; a chat spawner's is its own)",
        ));
    };
    out.insert(
        fantastic_ai_core::remote_provider::PROVIDER_FIELD.into(),
        json!(provider),
    );
    if let Some(m) = a.model.clone().or_else(|| own("model")) {
        out.insert("model".into(), json!(m));
    }
    Ok(out)
}

/// The provider a child completes through, from its chat record.
fn provider_of(fields: &Map<String, Value>) -> Option<String> {
    fields
        .get(fantastic_ai_core::remote_provider::PROVIDER_FIELD)
        .and_then(Value::as_str)
        .map(String::from)
}

/// Let `child` complete through `provider`: the provider admits it (a local provider; a
/// peer's is opened on its own kernel). Its own end is in its `can_call`.
fn open_provider(kernel: &Kernel, provider: &str, child: &str) {
    let id = AgentId::from(provider);
    if agent_of(kernel, &id).is_some() {
        let cid = child.to_string();
        let _ = kernel.edit_callable_by(&id, move |m| {
            union(
                m,
                &Map::from_iter([(String::from(&cid), json!(PROVIDER_VERBS))]),
            );
            true
        });
    }
}

/// The child's grants from its card: `$registry` made real.
fn card_can_call(reg: &AgentId, card: &Card) -> Map<String, Value> {
    let mut can_call = Map::new();
    for (k, v) in &card.grants {
        let key = if k == REGISTRY_TOKEN {
            reg.as_str()
        } else {
            k.as_str()
        };
        union(
            &mut can_call,
            &Map::from_iter([(key.to_string(), v.clone())]),
        );
    }
    can_call
}

/// What the registry lets the child call on it: what the card grants on
/// `$registry`, or nothing (no edge) when the card grants nothing there.
fn reg_verbs_of(can_call: &Map<String, Value>, reg: &AgentId) -> Option<Value> {
    can_call.get(reg.as_str()).cloned()
}

/// A failed grant write, in the registry's refusal shape.
fn grant_failed(e: String) -> Value {
    refusal::refusal_hint(
        refusal::GRANT_FAILED,
        format!("spawn: {e}"),
        "check that the agent records are writable, then spawn again",
    )
}

/// The edges every spawn opens: the registry accepts the child's calls (what the
/// card grants on `$registry`), the caller (when there is one) controls the child,
/// and the child may `send` the caller (its parent), on both ends, so it tells its
/// parent with a message that starts the parent's next turn. Every grant is MERGED
/// with what its holder already has (a host's reach may have written `*` there
/// before `create_agent` returned), never overwritten.
fn open_edges(
    kernel: &Kernel,
    reg: &AgentId,
    caller: Option<&AgentId>,
    child: &str,
    reg_verbs: Option<&Value>,
) -> Result<(), Value> {
    if let Some(verbs) = reg_verbs.cloned() {
        let child_id = child.to_string();
        kernel
            .edit_callable_by(reg, move |m| {
                union(m, &Map::from_iter([(child_id.clone(), verbs.clone())]));
                true
            })
            .map_err(grant_failed)?;
    }
    let Some(c) = caller else {
        return Ok(());
    };
    let child_id = AgentId::from(child);
    // the parent controls the child ...
    let cid = child.to_string();
    kernel
        .edit_can_call(c, move |m| {
            union(
                m,
                &Map::from_iter([(String::from(&cid), json!(PARENT_VERBS))]),
            );
            true
        })
        .map_err(grant_failed)?;
    // ... and the child tells the parent: its own `can_call` ...
    let pid = c.as_str().to_string();
    kernel
        .edit_can_call(&child_id, move |m| {
            union(
                m,
                &Map::from_iter([(String::from(&pid), json!(CHILD_VERBS))]),
            );
            true
        })
        .map_err(grant_failed)?;
    // ... and the parent's `callable_by`
    let cid = child.to_string();
    kernel
        .edit_callable_by(c, move |m| {
            union(
                m,
                &Map::from_iter([(String::from(&cid), json!(CHILD_VERBS))]),
            );
            true
        })
        .map_err(grant_failed)?;
    Ok(())
}

/// The child's parent: its place in the tree.
fn parent_of(kernel: &Kernel, child: &AgentId) -> Option<String> {
    let a = kernel.agents.get(child)?;
    a.value().parent_id.as_ref().map(|p| p.as_str().to_string())
}

/// The TEAM NOTE after a spawned child's task: the task comes FROM this registry, so
/// without it a child cannot tell whom to report to. Its parent by id, and the agents
/// on spawnable bundles under the same parent (its siblings) with their bundles.
fn team_note(kernel: &Kernel, reg: &AgentId, child: &AgentId) -> String {
    let Some(parent) = parent_of(kernel, child) else {
        return String::new();
    };
    let siblings: Vec<String> = spawned(kernel)
        .into_iter()
        .filter(|s| s.id != *child && parent_of(kernel, &s.id).as_deref() == Some(parent.as_str()))
        .map(|s| format!("`{}` ({})", s.id.as_str(), s.bundle))
        .collect();
    let mut note = format!(
        "\n\n[team] Your parent is `{parent}`: send it your result when you are done, with \
         <call name=\"send\"><routing><target_id>{parent}</target_id></routing><payload><type>send</type><text>…</text></payload></call>. \
         `{}` only delivered this task; it takes no reports.",
        reg.as_str()
    );
    if !siblings.is_empty() {
        note.push_str(&format!(" Your siblings: {}.", siblings.join(", ")));
    }
    note
}

fn send_first(kernel: &Arc<Kernel>, reg: &AgentId, child: &str, text: String) {
    let text = format!("{text}{}", team_note(kernel, reg, &AgentId::from(child)));
    let cid = child.to_string();
    let _ = kernel.edit_can_call(reg, move |m| {
        union(m, &Map::from_iter([(String::from(&cid), json!(["send"]))]));
        true
    });
    let k = Arc::clone(kernel);
    let target = AgentId::from(child);
    fantastic_kernel::spawn_inherit(async move {
        let _ = k
            .send(vec![Item::new(
                target,
                json!({"type": "send", "text": text}),
            )])
            .await;
    });
}

/// Everything `spawn` has settled before it branches from a fork point.
struct SpawnFrom<'a> {
    reg: &'a AgentId,
    card: &'a Card,
    caller: &'a Option<AgentId>,
    first_message: Option<String>,
}

/// The card's grants on tools: literal local agent ids only (not `$registry`, not
/// `$self`-like tokens, not `<guid>|x` peer keys).
fn tool_grants(card: &Card) -> Vec<(String, Value)> {
    card.grants
        .iter()
        .filter(|(k, _)| !k.starts_with('$') && !k.contains('|'))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// A card may only grant what its spawner holds: `None` when there is no caller (the
/// operator) or every verb the card lists on a tool is in the caller's own `can_call`;
/// otherwise the `not_allowed` refusal. `"*"` needs the caller to hold `"*"`.
fn escalation(kernel: &Kernel, caller: &Option<AgentId>, card: &Card) -> Option<Value> {
    let who = caller.as_ref()?;
    let holder = agent_of(kernel, who);
    let grant = holder
        .as_ref()
        .map(|w| meta_of(w).get("can_call").cloned().unwrap_or(Value::Null))
        .unwrap_or(Value::Null);
    let parent = holder
        .as_ref()
        .and_then(|w| w.parent_id.as_ref().map(|p| p.as_str().to_string()));
    for (tool, verbs) in tool_grants(card) {
        let listed: Vec<String> = match &verbs {
            Value::String(s) => vec![s.clone()],
            Value::Array(a) => a
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            _ => Vec::new(),
        };
        let ancestors =
            kernel.ancestor_ids(agent_of(kernel, &AgentId::from(tool.as_str())).as_deref());
        for verb in &listed {
            let held = fantastic_kernel::authz::authorize_call(
                &grant,
                verb,
                &tool,
                who.as_str(),
                parent.as_deref(),
                &ancestors,
            )
            .allowed;
            if !held {
                return Some(refusal::refusal_hint(
                    refusal::NOT_ALLOWED,
                    format!(
                        "spawn: {who} may not grant {} on {} (bundle {})",
                        py_repr(verb),
                        py_repr(&tool),
                        py_repr(&card.name)
                    ),
                    "drop that grant from the card, or spawn from an agent that holds it",
                ));
            }
        }
    }
    None
}

/// Admit `child` on the tools its card grants (the tool's end of the edge; the
/// child's own end is its `can_call`). A tool that is not an agent is skipped. All or
/// nothing: on a refused edit the edits made are taken back.
fn open_tool_edges(kernel: &Kernel, child: &str, card: &Card) -> Result<(), Value> {
    let mut done: Vec<AgentId> = Vec::new();
    for (tool, verbs) in tool_grants(card) {
        let id = AgentId::from(tool.as_str());
        if agent_of(kernel, &id).is_none() {
            continue;
        }
        let cid = child.to_string();
        match kernel.edit_callable_by(&id, move |m| {
            union(m, &Map::from_iter([(cid.clone(), verbs.clone())]));
            true
        }) {
            Ok(_) => done.push(id),
            Err(e) => {
                tool_ungrant(kernel, child, &done);
                return Err(grant_failed(e));
            }
        }
    }
    Ok(())
}

/// Take `child` back out of the `callable_by` of `tools`.
fn tool_ungrant(kernel: &Kernel, child: &str, tools: &[AgentId]) {
    for t in tools {
        let cid = child.to_string();
        let _ = kernel.edit_callable_by(t, move |m| m.remove(&cid).is_some());
    }
}

/// Take a half-made spawn back: its tool edges, its keys on the registry, the memory
/// store and the caller, then the agent itself.
async fn undo_spawn(
    kernel: &Arc<Kernel>,
    reg: &AgentId,
    caller: Option<&AgentId>,
    store: Option<&str>,
    card: &Card,
    child: &str,
) {
    let tools: Vec<AgentId> = tool_grants(card)
        .into_iter()
        .map(|(t, _)| AgentId::from(t.as_str()))
        .filter(|t| agent_of(kernel, t).is_some())
        .collect();
    tool_ungrant(kernel, child, &tools);
    if let Some(s) = store {
        let cid = child.to_string();
        let _ = kernel.edit_callable_by(&AgentId::from(s), move |m| m.remove(&cid).is_some());
    }
    let cid = child.to_string();
    let _ = kernel.edit_callable_by(reg, move |m| m.remove(&cid).is_some());
    if let Some(c) = caller {
        let cid = child.to_string();
        let _ = kernel.edit_can_call(c, move |m| m.remove(&cid).is_some());
        let cid = child.to_string();
        let _ = kernel.edit_callable_by(c, move |m| m.remove(&cid).is_some());
    }
    if let Some(a) = agent_of(kernel, &AgentId::from(child)) {
        fantastic_kernel::lifecycle::cascade_delete(kernel, &a).await;
    }
}

/// Does `who` hold `fork` on `source` in its own `can_call`?
fn holds_fork(kernel: &Kernel, who: &AgentId, source: &Agent) -> bool {
    let Some(w) = agent_of(kernel, who) else {
        return false;
    };
    let grant = meta_of(&w).get("can_call").cloned().unwrap_or(Value::Null);
    fantastic_kernel::authz::authorize_call(
        &grant,
        "fork",
        source.id.as_str(),
        who.as_str(),
        w.parent_id.as_ref().map(AgentId::as_str),
        &kernel.ancestor_ids(Some(source)),
    )
    .allowed
}

/// Let the registry `fork` the source: returns what to put back afterwards
/// (the registry's `can_call` entry for the source, the source's `callable_by`
/// entry for the registry; `None` inside = there was none).
fn fork_grant(
    kernel: &Kernel,
    reg: &AgentId,
    src: &AgentId,
) -> Result<(Option<Value>, Option<Value>), String> {
    let (mut before_cc, mut before_cb) = (None, None);
    let s = src.as_str().to_string();
    kernel.edit_can_call(reg, |m| {
        before_cc = m.get(&s).cloned();
        union(m, &Map::from_iter([(String::from(&s), json!(["fork"]))]));
        true
    })?;
    let r = reg.as_str().to_string();
    if let Err(e) = kernel.edit_callable_by(src, |m| {
        before_cb = m.get(&r).cloned();
        union(m, &Map::from_iter([(String::from(&r), json!(["fork"]))]));
        true
    }) {
        fork_ungrant(kernel, reg, src, &(before_cc, None));
        return Err(e);
    }
    Ok((before_cc, before_cb))
}

/// Take back what [`fork_grant`] added.
fn fork_ungrant(
    kernel: &Kernel,
    reg: &AgentId,
    src: &AgentId,
    before: &(Option<Value>, Option<Value>),
) {
    let put = |m: &mut Map<String, Value>, key: &str, old: &Option<Value>| {
        match old {
            Some(v) => m.insert(key.to_string(), v.clone()),
            None => m.remove(key),
        };
        true
    };
    let s = src.as_str().to_string();
    let old = before.0.clone();
    let _ = kernel.edit_can_call(reg, move |m| put(m, &s, &old));
    let r = reg.as_str().to_string();
    let old = before.1.clone();
    let _ = kernel.edit_callable_by(src, move |m| put(m, &r, &old));
}

/// `spawn {bundle, from:{agent, at}}`: `fork {at}` of `src` makes a CLONE of it — same
/// bundle, same record, same level (beside its source, in the source's store). Nothing is
/// written over the copy; the caller gets control of it and the registry may send its task.
async fn spawn_from(kernel: &Arc<Kernel>, c: &SpawnFrom<'_>, src: &str, at: i64) -> Value {
    let (reg, card) = (c.reg, c.card);
    let src_id = AgentId::from(src);
    let source = agent_of(kernel, &src_id);
    // a clone is of the source's own bundle
    if let Some(source) = source.as_ref() {
        if source.handler_module.as_deref() != Some(card.name.as_str()) {
            return refusal::refusal_hint(
                refusal::BAD_ARG,
                format!(
                    "spawn: a fork of {} is a clone of its bundle {}, not {}",
                    py_repr(src),
                    py_repr(source.handler_module.as_deref().unwrap_or("")),
                    py_repr(&card.name)
                ),
                "name the source's own bundle, or spawn without `from`",
            );
        }
    }
    // the caller (the operator when there is none) must itself hold `fork` on the source
    if let (Some(who), Some(source)) = (c.caller.as_ref(), source.as_ref()) {
        if !holds_fork(kernel, who, source) {
            return refusal::refusal_hint(
                refusal::NOT_ALLOWED,
                format!("spawn: {who} may not fork {}", py_repr(src)),
                "grant the spawner `fork` on that agent, or spawn without `from`",
            );
        }
    }
    let before = match fork_grant(kernel, reg, &src_id) {
        Ok(b) => b,
        Err(_) if source.is_none() => (None, None),
        Err(e) => return grant_failed(e),
    };
    // the registry forks AS ITSELF; whatever the fork refuses passes through unchanged
    let out = kernel
        .send(vec![Item::new(
            src_id.clone(),
            json!({"type": "fork", "at": at}),
        )])
        .await
        .remove(0)
        .result;
    fork_ungrant(kernel, reg, &src_id, &before);
    let Some(child) = out.get("id").and_then(Value::as_str).map(str::to_string) else {
        return out;
    };
    let copied = out.get("copied").cloned().unwrap_or(Value::Null);
    let Some(fork) = agent_of(kernel, &AgentId::from(child.as_str())) else {
        return out;
    };

    // the clone keeps the source's whole record; its store (for an undo) and its own
    // grants on the registry are read from it
    let fm = meta_of(&fork);
    let reg_verbs = fm
        .get("can_call")
        .and_then(Value::as_object)
        .and_then(|cc| reg_verbs_of(cc, reg));
    let store = fm
        .get("memory_store_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    // the caller controls it; the registry may `send` it its task
    let (caller, first) = (c.caller.clone(), c.first_message.is_some());
    let r = reg.as_str().to_string();
    let own = card.callable_by.clone();
    let opened = kernel
        .edit_callable_by(&fork.id, move |m| {
            union(m, &own);
            if let Some(w) = &caller {
                union(
                    m,
                    &Map::from_iter([(String::from(w.as_str()), json!(PARENT_VERBS))]),
                );
            }
            if first {
                union(m, &Map::from_iter([(String::from(&r), json!(["send"]))]));
            }
            true
        })
        .map_err(grant_failed)
        .and_then(|_| open_edges(kernel, reg, c.caller.as_ref(), &child, reg_verbs.as_ref()))
        .and_then(|()| open_tool_edges(kernel, &child, card));
    if let Err(e) = opened {
        // nothing of the registry is left behind
        undo_spawn(
            kernel,
            reg,
            c.caller.as_ref(),
            store.as_deref(),
            card,
            &child,
        )
        .await;
        return e;
    }
    if let Some(text) = c.first_message.clone() {
        send_first(kernel, reg, &child, text);
    }
    json!({
        "id": child, "bundle": card.name, "model": fm.get("model").cloned().unwrap_or(Value::Null),
        "from": {"agent": src, "at": at}, "copied": copied,
    })
}

fn fleet_reply(kernel: &Kernel) -> Value {
    let mut bundles: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for s in spawned(kernel) {
        bundles
            .entry(s.bundle)
            .or_default()
            .push(json!({"id": s.id.as_str()}));
    }
    json!({"bundles": bundles})
}

fn get_count_reply(kernel: &Kernel, bundle: Option<&str>) -> Value {
    let running = spawned(kernel)
        .iter()
        .filter(|s| bundle.is_none_or(|b| s.bundle == b))
        .count();
    json!({"running": running})
}

#[cfg(test)]
mod tests;
