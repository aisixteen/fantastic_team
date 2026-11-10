//! The ids and grant helpers of the team's agents. There is no plan here and no
//! config file: a team is agent records, made the way fantastic makes any agent
//! (a recipe, `send` with `create_agent`, or a directory that already holds
//! `.fantastic/agents/`). What the team page and
//! the dispatcher need granted at run time (the chat verbs on a worker, the
//! control verbs, the cadencer's digest targets, the card views) is written
//! here, from what the live records say.

use std::collections::BTreeSet;
use std::sync::Arc;

use fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
use fantastic_kernel::{Item, Kernel};
use serde_json::{json, Value};

/// The root agent's id: the target of `create_agent` and the other kernel verbs.
pub const ROOT_ID: &str = DEFAULT_ROOT_ID;
/// The dispatcher's agent id: the team's main view.
pub const DISPATCHER_ID: &str = "dispatcher";
/// The worker view agent, under the dispatcher; it opens a worker's view.
pub const WORKER_VIEW_ID: &str = "worker_view";
/// The board agent, when a team has one.
pub const BOARD_ID: &str = "board";
/// The cadencer agent.
pub const CADENCER_ID: &str = "cadencer";
/// The roles registry's conventional agent id.
pub const ROLES_ID: &str = "roles";
/// The task registry's conventional agent id (`task_registry`, crates/task_registry).
pub const TASKS_ID: &str = "tasks";
/// The kernel verbs the dispatcher may call on the root to add and remove a worker
/// (`control` with `add_worker` / `remove_worker`) and to keep the groups lists current.
pub const DISPATCHER_KERNEL_VERBS: [&str; 3] = ["create_agent", "update_agent", "delete_agent"];
/// The verbs the page uses on a worker or a subagent: its chat (`history`, `send`,
/// `stop`, `status`, `watch`). Granted by [`grant_chat_to`]; never `*`.
pub const CHAT_VERBS: [&str; 5] = ["history", "send", "stop", "status", "watch"];

/// True when the agent `id` is live in the kernel. Read from the kernel's table,
/// not by a `reflect`: a team bundle need not answer `reflect` (the dispatcher
/// and worker view do not), so a reflect would call a live agent missing.
fn exists(kernel: &Arc<Kernel>, id: &str) -> bool {
    kernel
        .agents
        .contains_key(&fantastic_kernel::AgentId::from(id))
}

/// Grant the page's principal (the `web_ws` leg that serves the page) the chat verbs on
/// each worker in `workers` and on every agent whose parent is one of them (its
/// subagents), on both ends: the principal's `can_call` names the agent and the
/// agent's `callable_by` names the principal. An operator write made at each start, so
/// it is idempotent. An agent that is not live is skipped; a refusal is reported, not
/// fatal. Returns how many agents got the grant. An agent made after the start gets it
/// at the next start.
pub fn grant_chat_to(kernel: &Arc<Kernel>, workers: &[String], principal: &str) -> usize {
    if !exists(kernel, principal) {
        return 0;
    }
    let live: Vec<String> = workers
        .iter()
        .filter(|w| exists(kernel, w))
        .cloned()
        .collect();
    let mut targets: BTreeSet<String> = live.iter().cloned().collect();
    for e in kernel.agents.iter() {
        let a = e.value();
        if a.parent_id.as_ref().is_some_and(|p| live.contains(&p.0)) {
            targets.insert(a.id.0.clone());
        }
    }
    let verbs = json!(CHAT_VERBS);
    let view = fantastic_kernel::AgentId::from(principal);
    let mut granted = 0;
    for id in targets {
        let agent = fantastic_kernel::AgentId::from(id.as_str());
        let by_agent = kernel.edit_callable_by(&agent, |cb| {
            if cb.get(principal) == Some(&verbs) {
                return false;
            }
            cb.insert(principal.into(), verbs.clone());
            true
        });
        let by_view = kernel.edit_can_call(&view, |cc| {
            if cc.get(&id) == Some(&verbs) {
                return false;
            }
            cc.insert(id.clone(), verbs.clone());
            true
        });
        match (by_agent, by_view) {
            (Ok(_), Ok(_)) => granted += 1,
            (Err(e), _) | (_, Err(e)) => eprintln!("dispatcher chat: `{id}`: {e}"),
        }
    }
    granted
}

/// Let `principal` (the page's `web_ws` leg) call the agents of `ids` that have a `view`
/// record, on both ends: the agent's `callable_by` and the leg's `can_call` name the
/// verbs of its panel (`view.calls`) plus `get_html` and `watch`, never `*`. An id that is
/// not live or has no `view` is skipped. Idempotent. Returns how many agents got the grant.
pub fn grant_card_ids(kernel: &Arc<Kernel>, principal: &str, ids: &[String]) -> usize {
    if !exists(kernel, principal) {
        return 0;
    }
    let leg = fantastic_kernel::AgentId::from(principal);
    let mut granted = 0;
    for id in ids {
        let agent = fantastic_kernel::AgentId::from(id.as_str());
        let calls: Option<Vec<String>> = kernel.agents.get(&agent).and_then(|e| {
            let meta = e.meta.read().ok()?;
            let view = meta.get("view")?.as_object()?;
            let mut verbs: Vec<String> = view
                .get("calls")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            for v in ["get_html", "watch"] {
                if !verbs.iter().any(|x| x == v) {
                    verbs.push(v.to_string());
                }
            }
            verbs.retain(|v| v != "*");
            Some(verbs)
        });
        let Some(verbs) = calls else { continue };
        let verbs = json!(verbs);
        let by_agent = kernel.edit_callable_by(&agent, |cb| {
            if cb.get(principal) == Some(&verbs) {
                return false;
            }
            cb.insert(principal.into(), verbs.clone());
            true
        });
        let by_leg = kernel.edit_can_call(&leg, |cc| {
            if cc.get(id) == Some(&verbs) {
                return false;
            }
            cc.insert(id.clone(), verbs.clone());
            true
        });
        match (by_agent, by_leg) {
            (Ok(_), Ok(_)) => granted += 1,
            (Err(e), _) | (_, Err(e)) => eprintln!("dispatcher cards: `{id}`: {e}"),
        }
    }
    granted
}

/// How many history pages [`grant_cards`] follows.
const CARD_HISTORY_PAGES: usize = 50;

/// [`grant_card_ids`] for the cards one chat agent shows: its history is read (all pages,
/// as the operator) and the embedded hosts that are on show get the grant. A chat that
/// does not answer `history` grants nothing.
pub async fn grant_cards(kernel: &Arc<Kernel>, principal: &str, chat_agent: &str) -> usize {
    if !exists(kernel, principal) || !exists(kernel, chat_agent) {
        return 0;
    }
    let mut rows: Vec<Value> = Vec::new();
    let mut args = json!({"type": "history"});
    for _ in 0..CARD_HISTORY_PAGES {
        let mut outs = kernel.send(vec![Item::new(chat_agent, args.clone())]).await;
        let Some(reply) = outs.pop().map(|o| o.result) else {
            break;
        };
        if reply.get("error").is_some() {
            break;
        }
        rows.extend(reply["rows"].as_array().cloned().unwrap_or_default());
        match reply["next"].as_u64() {
            Some(n) => args = json!({"type": "history", "after": n}),
            None => break,
        }
    }
    grant_card_ids(kernel, principal, &fantastic_ai_core::embed::cards(&rows))
}

/// The verbs the dispatcher relays to a worker: `send` (hand_task) and the `control`
/// actions `start`, `pause` and `stop`. Granted by [`grant_control`]; never `*`.
pub const CONTROL_VERBS: [&str; 4] = ["send", "stop", "start", "pause"];

/// Grant the `dispatcher` agent the [`CONTROL_VERBS`] on each worker in `workers`, on
/// both ends: the dispatcher's `can_call` names the worker and the worker's
/// `callable_by` names the dispatcher. An operator write made at each start, so it is
/// idempotent. A worker that is not live is skipped; a refusal is reported, not fatal.
/// Returns how many workers hold the grant. A worker made after the start is granted by
/// `add_worker`.
pub fn grant_control(kernel: &Arc<Kernel>, workers: &[String]) -> usize {
    if !exists(kernel, DISPATCHER_ID) {
        return 0;
    }
    let verbs = json!(CONTROL_VERBS);
    let dispatcher = fantastic_kernel::AgentId::from(DISPATCHER_ID);
    let mut granted = 0;
    for id in workers.iter().filter(|w| exists(kernel, w)) {
        let agent = fantastic_kernel::AgentId::from(id.as_str());
        let by_agent = kernel.edit_callable_by(&agent, |cb| {
            if cb.get(DISPATCHER_ID) == Some(&verbs) {
                return false;
            }
            cb.insert(DISPATCHER_ID.into(), verbs.clone());
            true
        });
        let by_dispatcher = kernel.edit_can_call(&dispatcher, |cc| {
            if cc.get(id) == Some(&verbs) {
                return false;
            }
            cc.insert(id.clone(), verbs.clone());
            true
        });
        match (by_agent, by_dispatcher) {
            (Ok(_), Ok(_)) => granted += 1,
            (Err(e), _) | (_, Err(e)) => eprintln!("dispatcher control: `{id}`: {e}"),
        }
    }
    granted
}

/// Let `agent` call the team's store agent `store` (every verb): the store's
/// `callable_by` names it, merged under the record's lock so the other callers stay.
/// The worker's own `can_call` already names the store (`config::agent_record`). An
/// operator write, idempotent. `Err` when the store is not live or refuses the edit.
pub fn grant_store(kernel: &Arc<Kernel>, store: &str, agent: &str) -> Result<(), String> {
    if !exists(kernel, store) {
        return Err(format!("the store agent `{store}` does not exist"));
    }
    kernel
        .edit_callable_by(&fantastic_kernel::AgentId::from(store), |cb| {
            let all =
                |v: &Value| v == "*" || v.as_array().is_some_and(|a| a.iter().any(|x| x == "*"));
            if cb.get(agent).is_some_and(all) {
                return false;
            }
            cb.insert(agent.into(), json!("*"));
            true
        })
        .map(|_| ())
        .map_err(|e| format!("store grant for `{agent}`: {e}"))
}

/// The ids the roles registry's `fleet` lists under `project_lead` and `lead`: the
/// agents that get a digest. Pure; an error reply or any other shape gives none.
pub fn digest_leads(fleet: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for role in ["project_lead", "lead"] {
        for e in fleet["roles"][role].as_array().into_iter().flatten() {
            if let Some(id) = e.as_str().or_else(|| e["id"].as_str()) {
                if !out.iter().any(|o| o == id) {
                    out.push(id.to_string());
                }
            }
        }
    }
    out
}

/// Let the cadencer `send` to the given live agents, on both ends: the agent's
/// `callable_by[cadencer]` and the cadencer's `can_call[agent]`. A spawned agent's
/// `callable_by` names only its spawner, so the digest could not reach it otherwise.
/// An operator write; idempotent; a dead id is skipped. Returns how many agents got a
/// new grant.
pub fn grant_digest_targets(kernel: &Arc<Kernel>, ids: &[String]) -> usize {
    if !exists(kernel, CADENCER_ID) {
        return 0;
    }
    let verbs = json!(["send"]);
    let mut granted = 0;
    for id in ids.iter().filter(|i| exists(kernel, i)) {
        let by_agent =
            kernel.edit_callable_by(&fantastic_kernel::AgentId::from(id.as_str()), |cb| {
                if cb.get(CADENCER_ID) == Some(&verbs) {
                    return false;
                }
                cb.insert(CADENCER_ID.into(), verbs.clone());
                true
            });
        let by_cadencer =
            kernel.edit_can_call(&fantastic_kernel::AgentId::from(CADENCER_ID), |cc| {
                if cc.get(id.as_str()) == Some(&verbs) {
                    return false;
                }
                cc.insert(id.clone(), verbs.clone());
                true
            });
        match (by_agent, by_cadencer) {
            (Ok(a), Ok(b)) => granted += usize::from(a || b),
            (Err(e), _) | (_, Err(e)) => eprintln!("cadencer digest target {id}: {e}"),
        }
    }
    granted
}

/// [`grant_digest_targets`] for the leads the roles registry `roles_id` lists now.
pub async fn grant_fleet_digest_targets(kernel: &Arc<Kernel>, roles_id: &str) -> usize {
    let outs = kernel
        .send(vec![Item::new(roles_id, json!({"type": "fleet"}))])
        .await;
    let fleet = outs
        .first()
        .map(|o| o.result.clone())
        .unwrap_or(Value::Null);
    grant_digest_targets(kernel, &digest_leads(&fleet))
}

// A2: boots through boot_kernel; switch on again
#[cfg(any())]
mod tests {
    use super::*;
    use crate::dispatcher;
    use fantastic_kernel::bootstrap::{self, DEFAULT_ROOT_ID};
    use fantastic_kernel::Status;
    use std::path::Path;

    #[test]
    fn digest_leads_lists_project_leads_and_leads_only() {
        let fleet = json!({"roles": {
            "project_lead": [{"id": "pl"}], "lead": [{"id": "sub"}, "sub2"],
            "worker": [{"id": "w1"}]}});
        assert_eq!(digest_leads(&fleet), ["pl", "sub", "sub2"]);
        assert!(digest_leads(&json!({"error": "down"})).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn grant_digest_targets_grants_send_to_fleet_leads() {
        let dir = tempfile::tempdir().unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        for id in [CADENCER_ID, "pl", "sub", "w1"] {
            let mk = json!({"type": "create_agent", "id": id, "parent_id": DEFAULT_ROOT_ID,
                "handler_module": "ram_store.tools", "callable_by": {"$self": "*", "$parent": "*"}});
            kernel.send(vec![Item::new(DEFAULT_ROOT_ID, mk)]).await;
        }
        let fleet = json!({"roles": {"project_lead": [{"id": "pl"}], "lead": [{"id": "sub"}, {"id": "gone"}],
            "worker": [{"id": "w1"}]}});
        let ids = digest_leads(&fleet);
        assert_eq!(
            grant_digest_targets(&kernel, &ids),
            2,
            "the dead id is skipped"
        );
        assert_eq!(grant_digest_targets(&kernel, &ids), 0, "idempotent");
        for id in ["pl", "sub"] {
            assert_eq!(
                field_of(&kernel, id, "callable_by")[CADENCER_ID],
                json!(["send"])
            );
            assert_eq!(
                field_of(&kernel, CADENCER_ID, "can_call")[id],
                json!(["send"])
            );
        }
        assert!(field_of(&kernel, "w1", "callable_by")[CADENCER_ID].is_null());
        assert!(field_of(&kernel, CADENCER_ID, "can_call")["w1"].is_null());
        assert!(field_of(&kernel, CADENCER_ID, "can_call")["gone"].is_null());
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[test]
    fn chat_verbs_are_the_chat_ones_only() {
        assert_eq!(CHAT_VERBS, ["history", "send", "stop", "status", "watch"]);
        assert!(!CHAT_VERBS.contains(&"*"));
    }

    /// A kernel with a bare dispatcher and workers `w1` and `w2`, none of them granted.
    async fn kernel_with_workers(dir: &Path) -> Arc<Kernel> {
        let kernel = crate::start(dir).await.unwrap();
        let mut items = vec![json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID,
            "id": "dispatcher", "handler_module": dispatcher::HANDLER_MODULE})];
        for w in ["w1", "w2"] {
            items.push(json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID,
                "id": w, "handler_module": "ram_store.tools"}));
        }
        for rec in items {
            let outs = kernel.send(vec![Item::new(DEFAULT_ROOT_ID, rec)]).await;
            assert!(
                matches!(outs[0].status, Status::Completed),
                "{}",
                outs[0].result
            );
        }
        kernel
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn grant_control_lets_the_dispatcher_send_and_stop_a_worker() {
        let dir = tempfile::tempdir().unwrap();
        let kernel = kernel_with_workers(dir.path()).await;
        let stop = json!({"type": "control", "node": "w1", "action": "stop", "confirm": true});
        let refused = |out: &Value| out["error"].as_str().unwrap_or("").contains("may not call");
        let before = kernel
            .send(vec![Item::new("dispatcher", stop.clone())])
            .await;
        assert!(refused(&before[0].result), "{}", before[0].result);

        assert_eq!(grant_control(&kernel, &["w1".to_string()]), 1);
        let after = kernel.send(vec![Item::new("dispatcher", stop)]).await;
        assert!(
            !refused(&after[0].result),
            "the edge is open now: {}",
            after[0].result
        );
        let grants = |id: &str, field: &str| {
            kernel
                .agents
                .get(&fantastic_kernel::AgentId::from(id))
                .map(|a| a.meta.read().unwrap()[field].clone())
                .unwrap_or(Value::Null)
        };
        assert_eq!(
            grants("w1", "callable_by")[DISPATCHER_ID],
            json!(CONTROL_VERBS)
        );
        assert_eq!(
            grants(DISPATCHER_ID, "can_call")["w1"],
            json!(CONTROL_VERBS)
        );
        // only the listed verbs, never `*`; and a worker that was not named stays closed
        for verb in ["delete_agent", "update_agent", "*"] {
            assert!(!CONTROL_VERBS.contains(&verb), "{verb}");
        }
        assert!(grants(DISPATCHER_ID, "can_call")["w2"].is_null());
        assert!(grants("w2", "callable_by")[DISPATCHER_ID].is_null());
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn grant_control_is_idempotent_and_skips_dead_workers() {
        let dir = tempfile::tempdir().unwrap();
        let kernel = kernel_with_workers(dir.path()).await;
        let ids = ["w1".to_string(), "nobody".to_string()];
        assert_eq!(grant_control(&kernel, &ids), 1, "the dead id is skipped");
        let snapshot = |id: &str| {
            kernel
                .agents
                .get(&fantastic_kernel::AgentId::from(id))
                .map(|a| {
                    let m = a.meta.read().unwrap();
                    (m["can_call"].clone(), m["callable_by"].clone())
                })
                .unwrap()
        };
        let (d1, w1) = (snapshot(DISPATCHER_ID), snapshot("w1"));
        assert_eq!(grant_control(&kernel, &ids), 1);
        assert_eq!((snapshot(DISPATCHER_ID), snapshot("w1")), (d1, w1));
        assert!(snapshot(DISPATCHER_ID).0.get("nobody").is_none());
        // no dispatcher, nothing granted
        kernel
            .send(vec![Item::new(
                DEFAULT_ROOT_ID,
                json!({"type": "delete_agent", "id": "dispatcher"}),
            )])
            .await;
        assert_eq!(grant_control(&kernel, &ids), 0);
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    // ---- the page leg and the card views ----

    /// A kernel with a live principal for the page's ws leg.
    async fn started_with_leg(dir: &Path) -> Arc<Kernel> {
        let kernel = crate::start(dir).await.unwrap();
        let leg = json!({"type": "create_agent", "id": crate::page::WS_ID, "parent_id": DEFAULT_ROOT_ID, "handler_module": "ram_store.tools"});
        let outs = kernel.send(vec![Item::new(DEFAULT_ROOT_ID, leg)]).await;
        assert!(
            matches!(outs[0].status, Status::Completed),
            "{}",
            outs[0].result
        );
        kernel
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_page_leg_keeps_its_chat_grant() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = started_with_leg(dir.path()).await;
        for rec in [
            json!({"type": "create_agent", "id": "w1", "parent_id": DEFAULT_ROOT_ID, "handler_module": "ram_store.tools"}),
            json!({"type": "create_agent", "id": "w1_sub", "parent_id": "w1", "handler_module": "ram_store.tools"}),
        ] {
            let outs = kernel.send(vec![Item::new(DEFAULT_ROOT_ID, rec)]).await;
            assert!(
                matches!(outs[0].status, Status::Completed),
                "{}",
                outs[0].result
            );
        }
        let granted = grant_chat_to(&kernel, &["w1".to_string()], crate::page::WS_ID);
        assert_eq!(granted, 2);
        for worker in ["w1", "w1_sub"] {
            assert_eq!(
                field_of(&kernel, worker, "callable_by")[crate::page::WS_ID],
                json!(CHAT_VERBS),
                "{worker}"
            );
            assert_eq!(
                field_of(&kernel, crate::page::WS_ID, "can_call")[worker],
                json!(CHAT_VERBS),
                "{worker}"
            );
        }
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    fn rows_of_cards(spec: &[(&str, &str)]) -> Vec<Value> {
        let mut rows = vec![json!({"role": "user", "head": "hi"})];
        rows.extend(spec.iter().map(|(k, h)| json!({"kind": k, "head": h})));
        rows
    }

    /// A host with an html panel that calls `get_html` and `x`.
    async fn panel_host(kernel: &Arc<Kernel>, id: &str, view: Value) {
        let mut rec = json!({"type": "create_agent", "id": id, "parent_id": DEFAULT_ROOT_ID,
            "handler_module": "ram_store.tools"});
        if !view.is_null() {
            rec["view"] = view;
        }
        let outs = kernel.send(vec![Item::new(DEFAULT_ROOT_ID, rec)]).await;
        assert!(
            matches!(outs[0].status, Status::Completed),
            "{}",
            outs[0].result
        );
    }

    fn field_of(kernel: &Arc<Kernel>, id: &str, f: &str) -> Value {
        kernel
            .agents
            .get(&fantastic_kernel::AgentId::from(id))
            .map(|a| a.meta.read().unwrap()[f].clone())
            .unwrap_or(Value::Null)
    }

    const CARD_VIEW: &str =
        r#"{"kind": "html", "bytes_file": "p.html", "calls": ["get_html", "x"]}"#;

    #[tokio::test(flavor = "multi_thread")]
    async fn grant_cards_lets_the_page_call_an_embedded_view_and_nothing_else() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = started_with_leg(dir.path()).await;
        let leg = crate::page::WS_ID;
        panel_host(&kernel, "h", serde_json::from_str(CARD_VIEW).unwrap()).await;
        panel_host(&kernel, "other", serde_json::from_str(CARD_VIEW).unwrap()).await;
        let ids = fantastic_ai_core::embed::cards(&rows_of_cards(&[("embed", "h")]));
        assert_eq!(grant_card_ids(&kernel, leg, &ids), 1);
        assert_eq!(
            field_of(&kernel, "h", "callable_by")[leg],
            json!(["get_html", "x", "watch"])
        );
        assert_eq!(
            field_of(&kernel, leg, "can_call")["h"],
            json!(["get_html", "x", "watch"])
        );
        assert_ne!(field_of(&kernel, "h", "callable_by")[leg], json!("*"));
        // a host that is not embedded is not granted
        assert!(field_of(&kernel, "other", "callable_by").get(leg).is_none());
        assert!(field_of(&kernel, leg, "can_call").get("other").is_none());
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn grant_cards_skips_an_unembedded_card() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = started_with_leg(dir.path()).await;
        panel_host(&kernel, "h", serde_json::from_str(CARD_VIEW).unwrap()).await;
        let ids =
            fantastic_ai_core::embed::cards(&rows_of_cards(&[("embed", "h"), ("unembed", "h")]));
        assert!(ids.is_empty());
        assert_eq!(grant_card_ids(&kernel, crate::page::WS_ID, &ids), 0);
        assert!(field_of(&kernel, "h", "callable_by")
            .get(crate::page::WS_ID)
            .is_none());
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn grant_cards_skips_an_agent_without_a_view() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = started_with_leg(dir.path()).await;
        panel_host(&kernel, "plain", Value::Null).await;
        let ids = vec!["plain".to_string(), "missing".to_string()];
        assert_eq!(grant_card_ids(&kernel, crate::page::WS_ID, &ids), 0);
        assert!(field_of(&kernel, "plain", "callable_by")
            .get(crate::page::WS_ID)
            .is_none());
        // a chat agent that does not answer history grants nothing and does not panic
        assert_eq!(grant_cards(&kernel, crate::page::WS_ID, "plain").await, 0);
        assert_eq!(grant_cards(&kernel, crate::page::WS_ID, "missing").await, 0);
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn grant_cards_is_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = started_with_leg(dir.path()).await;
        panel_host(&kernel, "h", serde_json::from_str(CARD_VIEW).unwrap()).await;
        let ids = vec!["h".to_string()];
        grant_card_ids(&kernel, crate::page::WS_ID, &ids);
        let first = (
            field_of(&kernel, "h", "callable_by"),
            field_of(&kernel, crate::page::WS_ID, "can_call"),
        );
        grant_card_ids(&kernel, crate::page::WS_ID, &ids);
        let second = (
            field_of(&kernel, "h", "callable_by"),
            field_of(&kernel, crate::page::WS_ID, "can_call"),
        );
        assert_eq!(first, second);
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }
}
