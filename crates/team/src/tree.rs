//! The ids and grant helpers of the team's agents. There is no plan here and no
//! config file: a team is agent records, made the way fantastic makes any agent
//! (a recipe, `send` with `create_agent`, a blueprint folder through `import`, or
//! a directory that already holds `.fantastic/agents/`). What the team page and
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
/// The canvas the dispatcher is placed on.
pub const CANVAS_ID: &str = "canvas_page";
/// The worker view agent, under the dispatcher; it opens a worker's view.
pub const WORKER_VIEW_ID: &str = "worker_view";
/// The board agent, when a team has one.
pub const BOARD_ID: &str = "board";
/// The cadencer agent.
pub const CADENCER_ID: &str = "cadencer";
/// The roles registry's conventional agent id.
pub const ROLES_ID: &str = "roles";
/// The task registry's conventional agent id (fantastic's `task_registry`).
pub const TASKS_ID: &str = "tasks";
/// The kernel verbs the dispatcher may call on the root to add and remove a worker
/// (`control` with `add_worker` / `remove_worker`) and to keep the groups lists current.
pub const DISPATCHER_KERNEL_VERBS: [&str; 3] = ["create_agent", "update_agent", "delete_agent"];
/// The verbs the page uses on a worker or a subagent: its chat (`history`, `send`,
/// `stop`, `status`, `watch`). Granted by [`grant_chat_to`]; never `*`.
pub const CHAT_VERBS: [&str; 5] = ["history", "send", "stop", "status", "watch"];

/// The pack verbs (`pack`, `install`, `eval`): the CLI, MCP and `send` reach them.
pub const PACK_ID: &str = "pack";
/// The pack listing (`list`, `show`, `updates`).
pub const PACK_LISTING_ID: &str = "pack_listing";

/// True when the agent `id` is live in the kernel. Read from the kernel's table,
/// not by a `reflect`: a team bundle need not answer `reflect` (the dispatcher
/// and worker view do not), so a reflect would call a live agent missing.
fn exists(kernel: &Arc<Kernel>, id: &str) -> bool {
    kernel
        .agents
        .contains_key(&fantastic_kernel::AgentId::from(id))
}

/// The agent a record names in a field, as the id it has now. A blueprint import gives
/// every agent a fresh id (`github` becomes `github_1`) and leaves a plain string field
/// as it was written, so when no agent has the id `named`, the one agent that `me` may
/// call (a `can_call` key) and whose handler is `handler_module` stands for it. With
/// none or more than one, `named` is returned as written.
pub fn peer_named(
    kernel: &Arc<Kernel>,
    me: &fantastic_kernel::AgentId,
    named: &str,
    handler_module: &str,
) -> String {
    if exists(kernel, named) {
        return named.to_string();
    }
    let keys: Vec<String> = kernel
        .agents
        .get(me)
        .and_then(|a| {
            a.meta
                .read()
                .ok()?
                .get("can_call")?
                .as_object()
                .map(|m| m.keys().cloned().collect())
        })
        .unwrap_or_default();
    let mut hits: Vec<String> = keys
        .into_iter()
        .filter(|k| {
            kernel
                .agents
                .get(&fantastic_kernel::AgentId::from(k.as_str()))
                .is_some_and(|a| a.handler_module.as_deref() == Some(handler_module))
        })
        .collect();
    hits.sort();
    match hits.as_slice() {
        [one] => one.clone(),
        _ => named.to_string(),
    }
}

/// The one root (kernel) verb the pack agent may call: `list_agents`, to check
/// that the agents a pack is bound to are live before it is enabled.
pub const PACK_ROOT_VERBS: [&str; 1] = ["list_agents"];
/// The `user` agent (`user.tools`): it holds the blueprint verbs.
pub const USER_ID: &str = "user";
/// The one `user` verb the pack agent may call (`install --parent-id`).
pub const PACK_USER_VERBS: [&str; 1] = ["import_subtree"];
/// The one `board` verb the pack agent may call (`eval {from_shifts}`).
pub const PACK_BOARD_VERBS: [&str; 1] = ["board"];
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

/// The agent ids whose cards a chat shows, from its history digests (rows of kind `embed`
/// and `unembed` with the agent id as `head`, or `content` in a full row), oldest first:
/// an embed adds its agent once, an unembed removes it. The rule of ai_core's
/// `embed::cards`; a row without a string id is ignored.
pub fn card_ids(rows: &[Value]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in rows {
        let kind = r.get("kind").and_then(Value::as_str);
        if kind != Some("embed") && kind != Some("unembed") {
            continue;
        }
        let Some(id) = r
            .get("content")
            .or_else(|| r.get("head"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        out.retain(|c| c != id);
        if kind == Some("embed") {
            out.push(id.to_string());
        }
    }
    out
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
    grant_card_ids(kernel, principal, &card_ids(&rows))
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

#[cfg(test)]
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

    // ---- the pack agents, made from records (no plan) ----

    /// A kernel with the stub `lead` and the pack agents as records, the way a team
    /// that wants packs makes them: `files` (the file bridge at the team directory),
    /// `pack`, `user` (its `import_subtree`), `pack_listing`, and the root's
    /// `list_agents` for `pack`. `members` are further agents `pack` may call.
    async fn kernel_with_pack_agents(dir: &Path, members: &[&str]) -> Arc<Kernel> {
        let kernel = crate::start(dir).await.unwrap();
        let mut pack_can_call = serde_json::Map::new();
        pack_can_call.insert("files".into(), json!("*"));
        pack_can_call.insert("lead".into(), json!("*"));
        pack_can_call.insert(USER_ID.into(), json!(PACK_USER_VERBS));
        pack_can_call.insert(DEFAULT_ROOT_ID.into(), json!(PACK_ROOT_VERBS));
        for m in members {
            pack_can_call.insert(m.to_string(), json!("*"));
        }
        let tool = |id: &str, rec: Value| {
            let mut rec = rec;
            rec["type"] = json!("create_agent");
            rec["id"] = json!(id);
            rec["parent_id"] = json!("lead");
            rec
        };
        let pack_cb = json!({"$self": "*", "$parent": "*", "dispatcher": "*"});
        let records = [
            json!({"type": "create_agent", "id": "lead", "parent_id": DEFAULT_ROOT_ID,
                "handler_module": "ram_store.tools",
                "callable_by": {"$self": "*", "$parent": "*", (PACK_ID): "*"}}),
            tool(
                "files",
                json!({"handler_module": "file_bridge.tools", "root": dir.to_string_lossy(),
                    "allow": ["."], "ingress_rule": "allow_all_ingress",
                    "callable_by": {"$self": "*", "$parent": "*", (PACK_ID): "*", (PACK_LISTING_ID): "*"}}),
            ),
            tool(
                PACK_ID,
                json!({"handler_module": crate::pack_bundle::HANDLER_MODULE,
                    "can_call": pack_can_call, "callable_by": pack_cb}),
            ),
            tool(
                USER_ID,
                json!({"handler_module": fantastic_user::HANDLER_MODULE,
                    "callable_by": {"$self": "*", "$parent": "*", (PACK_ID): PACK_USER_VERBS}}),
            ),
            tool(
                PACK_LISTING_ID,
                json!({"handler_module": crate::pack_listing::HANDLER_MODULE,
                    "index_path": "packs/index.json", "templates_dir": "templates",
                    "store": "files", "can_call": {"files": "*"}, "callable_by": pack_cb}),
            ),
        ];
        for rec in records {
            let outs = kernel.send(vec![Item::new(DEFAULT_ROOT_ID, rec)]).await;
            assert!(
                matches!(outs[0].status, Status::Completed)
                    && outs[0].result.get("error").is_none(),
                "{}",
                outs[0].result
            );
        }
        // the root's end of `pack`'s `list_agents`
        kernel
            .edit_callable_by(&fantastic_kernel::AgentId::from(DEFAULT_ROOT_ID), |cb| {
                cb.insert(PACK_ID.into(), json!(PACK_ROOT_VERBS));
                true
            })
            .unwrap();
        kernel
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pack_listing_answers_list_on_an_empty_index() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("templates")).unwrap();
        std::fs::create_dir_all(dir.path().join("packs")).unwrap();
        std::fs::write(dir.path().join("packs/index.json"), r#"{"entries":[]}"#).unwrap();
        let kernel = kernel_with_pack_agents(dir.path(), &[]).await;
        let outs = kernel
            .send(vec![Item::new(PACK_LISTING_ID, json!({"type": "list"}))])
            .await;
        let reply = &outs[0].result;
        assert!(reply.get("error").is_none(), "{reply}");
        assert_eq!(reply["packs"], json!([]), "{reply}");
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn applied_pack_verb_reaches_files_peer() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("templates")).unwrap();
        std::fs::create_dir_all(dir.path().join("w")).unwrap();
        std::fs::write(
            dir.path().join("w/worker.json"),
            r#"{"name":"w","version":"1.0"}"#,
        )
        .unwrap();
        let kernel = kernel_with_pack_agents(dir.path(), &[]).await;
        let packed = kernel
            .send(vec![Item::new(
                PACK_ID,
                json!({"type": "pack", "worker": "w", "out": "pack"}),
            )])
            .await;
        assert!(
            packed[0].result.get("error").is_none(),
            "{}",
            packed[0].result
        );
        let outs = kernel
            .send(vec![Item::new(
                PACK_ID,
                json!({"type": "install", "pack": "pack", "into": "templates"}),
            )])
            .await;
        let reply = &outs[0].result;
        assert!(reply.get("error").is_none(), "{reply}");
        assert!(reply.get("files").is_some(), "{reply}");
        assert!(dir.path().join("templates/w-1.0/worker.json").exists());
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    /// The whole path on a real kernel: pack a worker with a blueprint, install it with
    /// `parent_id` (preview, then approve) and find the agent under the lead.
    #[tokio::test(flavor = "multi_thread")]
    async fn applied_pack_install_builds_the_blueprint_worker() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("templates")).unwrap();
        std::fs::create_dir_all(dir.path().join("w/blueprint")).unwrap();
        std::fs::write(
            dir.path().join("w/worker.json"),
            r#"{"name":"w","version":"1.0"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("w/blueprint/blueprint.yaml"),
            "agents:\n  - id: bpw\n    handler_module: ram_store.tools\n",
        )
        .unwrap();
        let kernel = kernel_with_pack_agents(dir.path(), &[]).await;
        let send = |p: Value| {
            let kernel = kernel.clone();
            async move {
                kernel
                    .send(vec![Item::new(PACK_ID, p)])
                    .await
                    .remove(0)
                    .result
            }
        };
        let packed = send(json!({"type": "pack", "worker": "w", "out": "pack"})).await;
        assert!(packed.get("error").is_none(), "{packed}");
        let preview = send(
            json!({"type": "install", "pack": "pack", "into": "templates", "parent_id": "lead"}),
        )
        .await;
        assert!(preview.get("error").is_none(), "{preview}");
        assert_eq!(preview["approval"]["approval_required"], true, "{preview}");
        assert!(!exists(&kernel, "bpw_1"), "a preview creates nothing");
        let built = send(json!({"type": "install", "pack": "pack", "into": "templates2", "parent_id": "lead", "approve": true})).await;
        assert!(built.get("error").is_none(), "{built}");
        assert_eq!(built["installed_as"], "worker", "{built}");
        assert_eq!(built["ids"]["bpw"], "bpw_1", "{built}");
        assert!(exists(&kernel, "bpw_1"));
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    /// The pack agents with one packed worker `w` (one eval, id `a`) and a callable member
    /// `w1`: what the MCP `send` and `reflect` legs reach.
    async fn pack_agents_with_one_eval() -> (tempfile::TempDir, Arc<Kernel>) {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("w/evals")).unwrap();
        std::fs::write(
            dir.path().join("w/worker.json"),
            r#"{"name":"w","version":"1.0"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("w/evals/a.yaml"),
            "- id: a\n  task: good\n  check: {contains: ok}\n",
        )
        .unwrap();
        let kernel = kernel_with_pack_agents(dir.path(), &["w1"]).await;
        // A group member, as the lead would create it: it names `pack` in `callable_by`.
        let made = kernel
            .send(vec![Item::new(
                DEFAULT_ROOT_ID,
                json!({"type": "create_agent", "parent_id": "lead", "id": "w1",
                       "handler_module": "ram_store.tools",
                       "callable_by": {"$self": "*", "$parent": "*", "pack": "*"}}),
            )])
            .await;
        assert!(made[0].result.get("error").is_none(), "{}", made[0].result);
        let packed = kernel
            .send(vec![Item::new(
                PACK_ID,
                json!({"type": "pack", "worker": "w", "out": "pack"}),
            )])
            .await;
        assert!(
            packed[0].result.get("error").is_none(),
            "{}",
            packed[0].result
        );
        (dir, kernel)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn applied_pack_eval_reaches_a_worker() {
        let (dir, kernel) = pack_agents_with_one_eval().await;
        for worker in ["w1", "lead"] {
            let outs = kernel
                .send(vec![Item::new(
                    PACK_ID,
                    json!({"type": "eval", "pack": "pack", "worker": worker, "timeout_s": 5}),
                )])
                .await;
            let reply = outs[0].result.to_string();
            assert!(!reply.contains("may not call"), "{worker}: {reply}");
            assert!(
                outs[0].result.get("per_eval").is_some(),
                "{worker}: {reply}"
            );
        }
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mcp_docs_of_the_pack_agent_lists_eval_with_its_args() {
        let (dir, kernel) = pack_agents_with_one_eval().await;
        // The pack agent has no `reflect` verb of its own, so the MCP reach of its
        // verb docs is the kernel `docs` verb (an MCP `send` to `kernel`); its
        // reply lists them under `verbs`.
        let outs = kernel
            .send(vec![Item::new(
                "kernel",
                json!({"type": "docs", "of": PACK_ID, "tier": "detailed"}),
            )])
            .await;
        let reply = &outs[0].result;
        assert!(reply.get("error").is_none(), "{reply}");
        let docs = &reply["verbs"];
        for verb in ["pack", "install", "eval"] {
            assert!(docs.get(verb).is_some(), "{verb} not listed: {reply}");
        }
        let eval = docs["eval"].as_str().unwrap();
        assert!(eval.contains("eval {pack, worker, timeout_s?"), "{eval}");
        assert!(eval.contains("pass rate"), "{eval}");
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mcp_send_eval_reply_has_the_documented_shape() {
        let (dir, kernel) = pack_agents_with_one_eval().await;
        // The item the MCP `send` tool builds from {target, payload}.
        let outs = kernel
            .send(vec![Item::new(
                PACK_ID,
                json!({"type": "eval", "pack": "pack", "worker": "w1", "timeout_s": 5}),
            )])
            .await;
        let r = &outs[0].result;
        assert!(r.get("error").is_none(), "{r}");
        for key in ["pass_rate", "passed", "total", "total_time_s", "total_cost"] {
            assert!(r.get(key).is_some(), "{key} missing: {r}");
        }
        for key in ["pass_rate", "passed", "total", "total_time_s"] {
            assert!(r[key].is_number(), "{key}: {r}");
        }
        assert_eq!(r["total"], 1, "{r}");
        let per = r["per_eval"].as_array().expect("per_eval array");
        assert_eq!(per.len(), 1, "{r}");
        assert_eq!(per[0]["id"], "a", "{r}");
        for key in ["passed", "reason", "elapsed_s", "cost"] {
            assert!(per[0].get(key).is_some(), "per_eval.{key} missing: {r}");
        }
        assert!(r["text"].as_str().unwrap().contains("pass rate"), "{r}");
        let m = &r["measured"];
        assert!(m.is_object(), "{r}");
        for key in [
            "pack_hash",
            "tasks",
            "successes",
            "success_rate",
            "cost_per_task",
            "measured_by",
        ] {
            assert!(m.get(key).is_some(), "measured.{key} missing: {r}");
        }
        assert_eq!(m["signed"], false, "{r}");
        // The verb-level error reaches the MCP caller through the same path.
        let bad = kernel
            .send(vec![Item::new(
                PACK_ID,
                json!({"type": "eval", "pack": "pack", "worker": "", "timeout_s": 5}),
            )])
            .await;
        assert!(bad[0].result["error"].is_string(), "{}", bad[0].result);
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
        let ids = card_ids(&rows_of_cards(&[("embed", "h")]));
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
        let ids = card_ids(&rows_of_cards(&[("embed", "h"), ("unembed", "h")]));
        assert!(ids.is_empty());
        assert_eq!(grant_card_ids(&kernel, crate::page::WS_ID, &ids), 0);
        assert!(field_of(&kernel, "h", "callable_by")
            .get(crate::page::WS_ID)
            .is_none());
        // a row with no string id is ignored
        let odd = vec![
            json!({"kind": "embed"}),
            json!({"kind": "embed", "head": 3}),
        ];
        assert!(card_ids(&odd).is_empty());
        // an embed again after the unembed shows it once
        let again = card_ids(&rows_of_cards(&[
            ("embed", "h"),
            ("unembed", "h"),
            ("embed", "h"),
            ("embed", "h"),
        ]));
        assert_eq!(again, ["h"]);
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
