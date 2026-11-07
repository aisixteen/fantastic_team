//! A new team in roles mode: the fixed agents of `crates/team/blueprint.yaml`, written by
//! the first run with fantastic's `seed_dir_with` ([`crate::hydrate`]) and rehydrated from
//! disk after, as fantastic rehydrates its own. The ids are fixed because the role cards
//! name the tool agents by id in their grants (fantastic's import would give every agent a
//! fresh id: `github` becomes `github_1`).
//!
//! The ids: the main lead [`LEAD`]; under it the roles registry [`ROLES`], the task
//! registry [`TASKS`], the queue [`QUEUE`], the board [`BOARD`], the GitHub connector
//! [`GITHUB`] and the worker's tools [`WORKER_EXEC`] and [`WORKER_FILES`]. The cards (the
//! files in `roles/` of the repo, embedded in [`crate::roles`]) are written next to the
//! kernel by [`write_cards`]; the registry reads them from its `role_dirs`.
//!
//! There is no code here that creates, updates or grants an agent: the blueprint is the
//! one declaration.

use std::future::Future;
use std::path::Path;

use serde_json::{json, Value};

use crate::roles;
use crate::tree::{BOARD_ID, ROLES_ID, ROOT_ID, TASKS_ID};

/// The main lead: the agent the person chats with.
pub const LEAD: &str = "lead";
/// The roles registry (`agentic_resources`, crates/agentic_resources).
pub const ROLES: &str = ROLES_ID;
/// The task registry (`task_registry`, crates/task_registry).
pub const TASKS: &str = TASKS_ID;
/// The queue store: tasks wait there and the board keeps its cache there.
pub const QUEUE: &str = "queue";
/// The board: the cache of the open issues and pull requests.
pub const BOARD: &str = BOARD_ID;
/// The GitHub connector.
pub const GITHUB: &str = "github";
/// The worker's restricted exec.
pub const WORKER_EXEC: &str = "worker_exec";
/// The worker's file bridge on the work folder.
pub const WORKER_FILES: &str = "worker_files";

/// The home agent the lead is made under.
pub const PARENT: &str = "user";
/// The home's AI store.
pub const MEMORY: &str = "memory";
/// The home's file bridge.
pub const FILES: &str = "files";
/// The home's credential store.
pub const CREDENTIALS: &str = "credentials";

/// What the lead may call on the root: wait for its children, make and remove a sidecar.
pub const LEAD_ROOT_VERBS: [&str; 4] =
    ["await_set", "await_cancel", "create_agent", "delete_agent"];
/// What the lead may call on the roles registry.
pub const LEAD_ROLES_VERBS: [&str; 5] = ["roles", "role", "spawn", "get_count", "fleet"];
/// What the lead may call on the board.
pub const LEAD_BOARD_VERBS: [&str; 2] = ["board", "rows"];

/// The canvas agent of the app (fantastic's `canvas --port`): a sidecar's canvas.
pub const CANVAS: &str = crate::tree::CANVAS_ID;
/// What the lead may call on the canvas: the cards of the workers and leads grant it too,
/// and a spawn grants only what the spawner holds.
pub const LEAD_CANVAS_VERBS: [&str; 1] = ["new_canvas"];
/// What the lead may call on the home's `files`, and what `files` admits from the lead: `read`,
/// and the verbs its `system_prompt_file` is read and followed with through its
/// `file_bridge_id` (fantastic's ai_core `prompt_file.rs`: `read_stream` at :290, `watch_path`
/// at :232 and :253, `unwatch_path` at :340, and the bridge `watch` at :239).
pub const LEAD_FILES_VERBS: [&str; 5] =
    ["read", "read_stream", "watch_path", "unwatch_path", "watch"];

/// Write the role cards under `dir/roles` (the folder the registry's `role_dirs` names,
/// relative to the directory the team runs in). A card the directory already has stays as
/// it is. Returns how many cards were written.
pub fn write_cards(dir: &Path) -> Result<usize, String> {
    let cards = dir.join("roles");
    std::fs::create_dir_all(&cards)
        .map_err(|e| format!("cannot create {}: {e}", cards.display()))?;
    let mut wrote = 0;
    for (file, text) in roles::ALL {
        let p = cards.join(file);
        if !p.exists() {
            std::fs::write(&p, text).map_err(|e| format!("cannot write {}: {e}", p.display()))?;
            wrote += 1;
        }
    }
    Ok(wrote)
}

/// The agent the key mode is asked and set through: fantastic's `setup.tools`, the agent its
/// set-up page talks to. It exists only for these calls.
pub const KEY_MODE_AGENT: &str = "key_mode_setup";

/// Fantastic makes its credential store (the agent `credentials`) only once a key mode is
/// chosen, and the team's grants name that store. When no mode is chosen yet, choose `plain`
/// (fantastic's own mode: the keys in one private file in the home's keys folder) through
/// fantastic's `set_key_mode`, the call its set-up page makes, on a `setup.tools` agent made
/// for it and deleted after. A mode the user chose stays as it is. It runs before the team is used.
/// Returns whether it chose `plain`.
pub async fn choose_key_mode<S, F>(mut send: S) -> Result<bool, String>
where
    S: FnMut(&'static str, Value) -> F,
    F: Future<Output = Result<Value, String>>,
{
    fn failed(what: &str, r: &Value) -> Result<(), String> {
        match r.get("error") {
            Some(e) => Err(format!("{what}: {e}")),
            None => Ok(()),
        }
    }
    let made = send(
        ROOT_ID,
        json!({"type": "create_agent", "parent_id": PARENT, "id": KEY_MODE_AGENT,
               "handler_module": "setup.tools"}),
    )
    .await?;
    failed("the key mode could not be asked", &made)?;
    let chosen: Result<bool, String> = async {
        let now = send(KEY_MODE_AGENT, json!({"type": "key_mode"})).await?;
        failed("the key mode could not be read", &now)?;
        if !now["mode"].is_null() {
            return Ok(false);
        }
        let set = send(
            KEY_MODE_AGENT,
            json!({"type": "set_key_mode", "mode": "plain"}),
        )
        .await?;
        failed("the key mode could not be set to plain", &set)?;
        Ok(true)
    }
    .await;
    let gone = send(
        ROOT_ID,
        json!({"type": "delete_agent", "id": KEY_MODE_AGENT}),
    )
    .await;
    let chosen = chosen?;
    failed("the key mode agent could not be removed", &gone?)?;
    Ok(chosen)
}

/// True when `list_agents`'s reply holds an agent with this id.
pub fn has_agent(list: &Value, id: &str) -> bool {
    list["agents"]
        .as_array()
        .is_some_and(|a| a.iter().any(|r| r["id"] == id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fantastic_kernel::{bootstrap, AgentId, Item, Kernel};
    use std::sync::Arc;

    #[test]
    fn write_cards_never_overwrites() {
        let dir = tempfile::TempDir::new().unwrap();
        assert_eq!(write_cards(dir.path()).unwrap(), roles::ALL.len());
        for (file, _) in roles::ALL {
            assert!(dir.path().join("roles").join(file).is_file(), "{file}");
        }
        let mine = dir.path().join("roles/worker.md");
        std::fs::write(&mine, "# mine\n").unwrap();
        assert_eq!(write_cards(dir.path()).unwrap(), 0);
        assert_eq!(std::fs::read_to_string(&mine).unwrap(), "# mine\n");
    }

    #[test]
    fn the_embedded_cards_equal_the_repo_cards() {
        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roles");
        for (file, text) in roles::ALL {
            let on_disk = std::fs::read_to_string(repo.join(file)).unwrap();
            assert_eq!(text, on_disk, "{file} drifted from roles/{file}");
        }
    }

    /// An agent that makes the call it is told to AS ITSELF, so the gate and `spawn`
    /// see a real lead (the lead and the strong tier are backends that need a model).
    struct CallerBundle(Arc<std::sync::Mutex<Vec<(String, Value)>>>);

    #[async_trait::async_trait]
    impl fantastic_kernel::bundle::Bundle for CallerBundle {
        fn name(&self) -> &str {
            "caller"
        }

        async fn handle(
            &self,
            agent_id: &AgentId,
            payload: &Value,
            kernel: &Arc<Kernel>,
        ) -> Result<fantastic_kernel::bundle::Reply, fantastic_kernel::bundle::BundleError>
        {
            match payload["type"].as_str() {
                // what a digest or a first message hands the agent
                Some("send") => {
                    self.0
                        .lock()
                        .unwrap()
                        .push((agent_id.as_str().to_string(), payload.clone()));
                    Ok(Some(json!({"ok": true})))
                }
                Some("call") => {
                    let out = kernel
                        .send(vec![Item::new(
                            payload["target"].as_str().unwrap_or(""),
                            payload["payload"].clone(),
                        )])
                        .await
                        .remove(0);
                    Ok(Some(out.result))
                }
                _ => Ok(None),
            }
        }
    }

    /// A home seeded the way the team's first run seeds one (fantastic's `seed_dir_with` over
    /// `crates/team/blueprint.yaml`), with this repo's bundles and a `caller.tools` bundle: the
    /// lead and the strong tier run on `caller.tools` (no model is called), nothing else
    /// changes.
    async fn team() -> (tempfile::TempDir, Arc<Kernel>) {
        let (dir, kernel, _) = team_with_log().await;
        (dir, kernel)
    }

    type SentLog = Arc<std::sync::Mutex<Vec<(String, Value)>>>;

    /// [`team`], and the log of the `send`s its `caller.tools` agents received.
    async fn team_with_log() -> (tempfile::TempDir, Arc<Kernel>, SentLog) {
        build(None).await
    }

    /// ai_core keeps process-wide state keyed by agent id (the in-flight turns, the
    /// backends, the prompt files), and every real lead is `lead`: the tests that run one
    /// take turns, as two apps in one process never would share an id.
    static REAL_LEAD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// The agent `id` of a blueprint document, wherever it sits.
    fn entry_mut<'a>(list: &'a mut Value, id: &str) -> Option<&'a mut Value> {
        for e in list.as_array_mut()? {
            if e["id"] == id {
                return Some(e);
            }
            if let Some(found) = e.get_mut("agents").and_then(|kids| entry_mut(kids, id)) {
                return Some(found);
            }
        }
        None
    }

    /// The blueprint these tests seed: the team's own, with the web host left out (these
    /// kernels serve nothing), the home's `credentials` store added under `user` (the
    /// store fantastic makes once a key mode is chosen), the lead on `caller.tools` or on
    /// `real_lead`'s endpoint, the strong tier on `caller.tools` and the registry reading
    /// the cards of this directory.
    fn test_blueprint(dir: &Path, real_lead: Option<&str>) -> String {
        let mut doc: Value =
            serde_yaml::from_str(&crate::hydrate::blueprint_text(dir).unwrap()).unwrap();
        let list = &mut doc["agents"];
        list.as_array_mut().unwrap().retain(|a| a["id"] != "web");
        let mut creds = json!({"id": CREDENTIALS, "handler_module": "credential_store.tools",
                               "backend": "env", "refs": {}});
        if let Some(endpoint) = real_lead {
            creds["backend"] = json!("file");
            creds["file"] = json!(dir.join("creds.yaml").to_string_lossy());
            let host = endpoint.trim_start_matches("http://");
            creds["refs"] = json!({"plug": {"host": host.split('/').next().unwrap()}});
        }
        entry_mut(list, PARENT).unwrap()["agents"]
            .as_array_mut()
            .unwrap()
            .push(creds);
        let lead = entry_mut(list, LEAD).unwrap();
        match real_lead {
            None => lead["handler_module"] = json!("caller.tools"),
            Some(endpoint) => {
                lead["endpoint"] = json!(endpoint);
                lead["credentials"] = json!({"store": CREDENTIALS, "ref": "plug"});
            }
        }
        let roles = entry_mut(list, ROLES).unwrap();
        roles["tiers"]["strong"]["handler_module"] = json!("caller.tools");
        // the cards are in this directory, not the one the test runs in
        roles["role_dirs"] = json!([dir.join("roles").to_string_lossy()]);
        serde_yaml::to_string(&doc).unwrap()
    }

    /// What fantastic's own boot does for an AI agent and a blueprint cannot: the other end
    /// of the lead's edges on the stock agents (`reach::grant_all`, fantastic
    /// `src/tui/src/kern/reach.rs`, run by `Host::boot` in the shell). These kernels are
    /// in-process, with no shell, so the test writes that end itself.
    fn reach_like_the_shell(kernel: &Arc<Kernel>) {
        for id in [MEMORY, FILES, CREDENTIALS, ROOT_ID] {
            kernel
                .edit_callable_by(&AgentId::from(id), |cb| {
                    cb.insert(LEAD.into(), json!("*"));
                    true
                })
                .unwrap();
        }
    }

    /// The home and the team of [`team_with_log`]. With `Some(endpoint)` the lead stays the
    /// real backend the app makes, pointed at `endpoint` (a scripted model on 127.0.0.1) with a
    /// key from a file credential store; nothing else about it changes.
    async fn build(real_lead: Option<&str>) -> (tempfile::TempDir, Arc<Kernel>, SentLog) {
        let log: SentLog = Arc::default();
        let dir = tempfile::TempDir::new().unwrap();
        write_cards(dir.path()).unwrap();
        fantastic_tui::seed_dir_with(dir.path(), &[&test_blueprint(dir.path(), real_lead)])
            .unwrap();
        let mut reg = crate::registry();
        reg.register("caller.tools", CallerBundle(log.clone()));
        let mut booted =
            bootstrap::bootstrap(reg, bootstrap::BootstrapOptions::one_shot(dir.path())).unwrap();
        fantastic_kernel::lock::acquire(dir.path()).unwrap();
        booted.write_seed().unwrap();
        let kernel = booted.kernel.clone();
        crate::boot_kernel(&kernel, true).await.unwrap();
        reach_like_the_shell(&kernel);
        if real_lead.is_some() {
            let o = ask(
                &kernel,
                "credentials",
                json!({"type": "set", "ref": "plug", "fields": {"api_key": "sk-plug"}}),
            )
            .await;
            assert_eq!(o["stored"], json!(true), "{o}");
        }
        (dir, kernel, log)
    }

    fn meta(kernel: &Arc<Kernel>, id: &str) -> Value {
        kernel
            .agents
            .get(&AgentId::from(id))
            .map(|a| Value::Object(a.meta.read().unwrap().clone()))
            .unwrap_or_else(|| panic!("no agent {id}"))
    }

    async fn ask(kernel: &Arc<Kernel>, target: &str, p: Value) -> Value {
        kernel
            .send(vec![Item::new(target, p)])
            .await
            .remove(0)
            .result
    }

    async fn call_as(kernel: &Arc<Kernel>, caller: &str, target: &str, p: Value) -> Value {
        ask(
            kernel,
            caller,
            json!({"type": "call", "target": target, "payload": p}),
        )
        .await
    }

    async fn finish(dir: &Path, kernel: Arc<Kernel>) {
        kernel.shutdown().await;
        fantastic_sqlite_store::close_all(&kernel);
        bootstrap::shutdown(dir).unwrap();
    }

    /// The app's lead takes its first turn: its `system_prompt_file` is read through `files`
    /// with the grants of [`steps`] alone (the model is scripted on 127.0.0.1, no real API),
    /// and what the model is sent is the card's text.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_leads_first_turn_reads_its_system_prompt_file() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let srv = MockServer::start().await;
        // The answer ends the chain with `done`. A call-less answer is nudged (fantastic's
        // ai_core `route.rs`, `nudging` and `deliver_soon`): the chain goes on after `send`
        // has replied, and a second request reaches the model whenever it lands before the
        // count is read (#250).
        let body: String = [
            json!({"type": "message_start", "message": {"usage": {"input_tokens": 10}}}),
            json!({"type": "content_block_delta", "index": 0,
                   "delta": {"type": "text_delta", "text": "ready<call name=\"done\"></call>"}}),
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"},
                   "usage": {"output_tokens": 1}}),
            json!({"type": "message_stop"}),
        ]
        .iter()
        .map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap()))
        .collect();
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .mount(&srv)
            .await;
        let _turn = REAL_LEAD.lock().await;
        let (dir, kernel, _) = build(Some(&format!("{}/v1/messages", srv.uri()))).await;
        let r = ask(&kernel, LEAD, json!({"type": "send", "text": "go"})).await;
        let err = meta(&kernel, LEAD)["system_prompt_file_error"].clone();
        assert!(
            err.is_null(),
            "the prompt file was refused: {err}; reply {r}"
        );
        assert!(!r.to_string().contains("may not call"), "{r}");
        let seen = srv.received_requests().await.unwrap_or_default();
        assert_eq!(seen.len(), 1, "the model got no request: {r}");
        let card = std::fs::read_to_string(dir.path().join("roles/lead.md")).unwrap();
        let sent: Value = serde_json::from_slice(&seen[0].body).unwrap();
        assert!(
            sent["system"]
                .to_string()
                .contains(card.lines().nth(2).unwrap()),
            "the system block is not the card: {}",
            sent["system"]
        );
        finish(dir.path(), kernel).await;
    }

    /// A spawned child tells its parent with a plain `send` (team#247): `spawn` opens the
    /// edge child -> parent on both ends, the gate admits the child's `send`, and the
    /// message starts the AI parent's turn (the app's real lead, its model scripted on
    /// 127.0.0.1). The spawn is the lead's own call (made as the lead, as its model's
    /// tool call would be); the child is a sub-lead on the strong tier.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_spawned_childs_send_starts_its_ai_parents_turn() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let srv = MockServer::start().await;
        // the answer ends the chain with `done`, so ai_core sends no no-call nudge: the
        // model is asked exactly once
        let body: String = [
            json!({"type": "message_start", "message": {"usage": {"input_tokens": 10}}}),
            json!({"type": "content_block_delta", "index": 0,
                   "delta": {"type": "text_delta",
                              "text": "noted <call name=\"done\"></call>"}}),
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"},
                   "usage": {"output_tokens": 1}}),
            json!({"type": "message_stop"}),
        ]
        .iter()
        .map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap()))
        .collect();
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .mount(&srv)
            .await;
        let _turn = REAL_LEAD.lock().await;
        let (dir, kernel, _) = build(Some(&format!("{}/v1/messages", srv.uri()))).await;
        let lead = AgentId::from(LEAD);
        let before = meta(&kernel, LEAD)["callable_by"].clone();
        let made = fantastic_kernel::send::as_agent(
            &lead,
            ask(&kernel, ROLES, json!({"type": "spawn", "role": "lead"})),
        )
        .await;
        let sub = made["id"]
            .as_str()
            .unwrap_or_else(|| panic!("spawn: {made}"))
            .to_string();
        assert_eq!(
            kernel
                .agents
                .get(&AgentId::from(sub.as_str()))
                .unwrap()
                .parent_id,
            Some(lead.clone()),
            "the sub-lead is the lead's child"
        );
        // both ends of the edge, merged with what each already held
        let held = |v: &Value| v.as_array().is_some_and(|a| a.contains(&json!("send")));
        assert!(
            held(&meta(&kernel, &sub)["can_call"][LEAD]),
            "{sub} may send the lead"
        );
        assert!(
            held(&meta(&kernel, LEAD)["callable_by"][sub.as_str()]),
            "the lead admits {sub}"
        );
        let after = meta(&kernel, LEAD)["callable_by"].clone();
        for (k, v) in before.as_object().unwrap() {
            assert_eq!(&after[k], v, "what the lead admitted before stays: {k}");
        }
        assert!(
            srv.received_requests().await.unwrap_or_default().is_empty(),
            "nothing has asked the lead's model yet"
        );
        let text = format!("{sub}: my branch is done, the test passes");
        let out = call_as(&kernel, &sub, LEAD, json!({"type": "send", "text": text})).await;
        assert!(out.get("error").is_none(), "the send is admitted: {out}");
        let seen = srv.received_requests().await.unwrap_or_default();
        assert_eq!(seen.len(), 1, "the message started one lead turn: {out}");
        let sent: Value = serde_json::from_slice(&seen[0].body).unwrap();
        assert!(
            sent["messages"].to_string().contains(&text),
            "the lead's turn carries the child's message: {}",
            sent["messages"]
        );
        finish(dir.path(), kernel).await;
    }

    /// Roles mode on the new path: the lead has the roles registry and its spawn grants,
    /// the registry is the tiers-and-limits record, and the home agents admit the lead.
    #[tokio::test(flavor = "multi_thread")]
    async fn roles_mode_the_lead_holds_the_registry_and_its_spawn_grants() {
        let (dir, kernel) = team().await;
        for id in [
            LEAD,
            QUEUE,
            GITHUB,
            BOARD,
            ROLES,
            TASKS,
            WORKER_EXEC,
            WORKER_FILES,
        ] {
            assert!(kernel.agents.contains_key(&AgentId::from(id)), "{id}");
        }
        let lead = meta(&kernel, LEAD);
        assert_eq!(lead["memory_store_id"], "memory");
        assert_eq!(lead["system_prompt_file"], "roles/lead.md");
        assert_eq!(lead["credentials"]["store"], "credentials");
        assert_eq!(lead["can_call"]["roles"], json!(LEAD_ROLES_VERBS));
        assert_eq!(lead["can_call"]["kernel_state"], json!(LEAD_ROOT_VERBS));
        assert_eq!(lead["can_call"]["board"], json!(LEAD_BOARD_VERBS));
        let reg = meta(&kernel, ROLES);
        assert_eq!(reg["tiers"]["small"]["model"], "claude-haiku-5-5");
        assert_eq!(reg["limits"]["total"], 3);
        assert_eq!(reg["callable_by"]["lead"], json!(LEAD_ROLES_VERBS));
        // the other ends on the home agents, merged with what they had
        assert_eq!(meta(&kernel, "memory")["callable_by"]["lead"], "*");
        assert!(
            meta(&kernel, "memory")["callable_by"].get("user").is_some(),
            "what the home agent had stays"
        );
        assert_eq!(meta(&kernel, "files")["callable_by"]["lead"], "*");
        assert_eq!(meta(&kernel, ROOT_ID)["callable_by"]["lead"], "*");
        // the registry reads the cards, all six
        let all = call_as(&kernel, LEAD, ROLES, json!({"type": "roles"})).await;
        for name in roles::CODER_ROLES {
            assert!(all.to_string().contains(name), "{name}: {all}");
        }
        finish(dir.path(), kernel).await;
    }

    /// The lead spawns through the registry as its card says: a worker with the tier's
    /// backend fields and the card's grants on the tools under their fixed ids, and a
    /// project_lead (whose card grants the connector, board, queue, tasks and the root),
    /// within the registry's limit of three.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_lead_spawns_a_worker_and_a_project_lead_through_the_registry() {
        let (dir, kernel) = team().await;
        let spawn = |role: &'static str| json!({"type": "spawn", "role": role});
        let w = call_as(&kernel, LEAD, ROLES, spawn("worker")).await;
        let worker = w["id"]
            .as_str()
            .unwrap_or_else(|| panic!("worker: {w}"))
            .to_string();
        assert_eq!(w["tier"], "small", "{w}");
        let wm = meta(&kernel, &worker);
        assert_eq!(wm["model"], "claude-haiku-5-5");
        assert_eq!(wm["memory_store_id"], "memory");
        assert_eq!(wm["spawned_by"], "roles");
        for tool in [WORKER_EXEC, WORKER_FILES, QUEUE, MEMORY] {
            assert_eq!(wm["can_call"][tool], "*", "{tool}: {wm:?}");
            assert!(
                meta(&kernel, tool)["callable_by"]
                    .get(worker.as_str())
                    .is_some(),
                "{tool} admits the worker"
            );
        }
        let p = call_as(&kernel, LEAD, ROLES, spawn("project_lead")).await;
        let pl = p["id"]
            .as_str()
            .unwrap_or_else(|| panic!("project_lead: {p}"))
            .to_string();
        let pm = meta(&kernel, &pl);
        for tool in [GITHUB, BOARD, QUEUE, TASKS, ROOT_ID] {
            assert!(pm["can_call"].get(tool).is_some(), "{tool}: {pm:?}");
        }
        let count = ask(&kernel, ROLES, json!({"type": "get_count"})).await;
        assert_eq!(count["running"], 2, "{count}");
        assert_eq!(count["max"], 3, "{count}");
        // the limit of the record is read: a third worker is one too many past the total
        let w2 = call_as(&kernel, LEAD, ROLES, spawn("worker")).await;
        assert!(w2["id"].is_string(), "{w2}");
        let w3 = call_as(&kernel, LEAD, ROLES, spawn("worker")).await;
        assert_eq!(w3["reason"], "limit", "{w3}");
        finish(dir.path(), kernel).await;
    }

    /// Room for every spawn of the tests below: the app's registry allows three agents in
    /// all, and these trees hold up to six. A record field the registry already has.
    async fn raise_total(kernel: &Arc<Kernel>, total: u64) {
        let o = ask(
            kernel,
            ROOT_ID,
            json!({"type": "update_agent", "id": ROLES, "limits": {"total": total}}),
        )
        .await;
        assert!(o.get("error").is_none(), "{o}");
    }

    async fn spawn_as(kernel: &Arc<Kernel>, caller: &str, role: &str) -> String {
        let r = call_as(
            kernel,
            caller,
            ROLES,
            json!({"type": "spawn", "role": role}),
        )
        .await;
        assert!(r.get("error").is_none(), "{caller} spawns {role}: {r}");
        r["id"]
            .as_str()
            .unwrap_or_else(|| panic!("{caller} spawns {role}: {r}"))
            .to_string()
    }

    /// The team's own cards: the main lead spawns a sub-lead (the `lead` card) and that
    /// sub-lead spawns a worker; the main lead spawns a project_lead and that project_lead
    /// spawns a worker. A card may only grant what its spawner holds (aisixteen/fantastic_team#227).
    #[tokio::test(flavor = "multi_thread")]
    async fn leads_and_project_leads_spawn_workers_with_the_team_cards() {
        let (dir, kernel) = team().await;
        raise_total(&kernel, 6).await;
        let sub = spawn_as(&kernel, LEAD, "lead").await;
        let w1 = spawn_as(&kernel, &sub, "worker").await;
        let pl = spawn_as(&kernel, LEAD, "project_lead").await;
        let w2 = spawn_as(&kernel, &pl, "worker").await;
        assert_eq!(meta(&kernel, &w1)["spawned_by"], "roles");
        assert_eq!(meta(&kernel, &w2)["spawned_by"], "roles");
        for w in [&w1, &w2] {
            assert_eq!(meta(&kernel, w)["can_call"][WORKER_EXEC], "*");
            assert_eq!(meta(&kernel, w)["can_call"][WORKER_FILES], "*");
        }
        let count = ask(&kernel, ROLES, json!({"type": "get_count"})).await;
        assert_eq!(count["running"], 4, "{count}");
        finish(dir.path(), kernel).await;
    }

    /// Every card in `roles/` can be spawned by the main lead, so a card cannot grow past
    /// the lead's grants without this test saying so.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_main_lead_can_spawn_every_card() {
        let (dir, kernel) = team().await;
        raise_total(&kernel, 12).await;
        for name in roles::CODER_ROLES {
            let r = call_as(&kernel, LEAD, ROLES, json!({"type": "spawn", "role": name})).await;
            assert!(r["id"].is_string(), "{name}: {r}");
            assert_ne!(r["reason"], "not_allowed", "{name}: {r}");
        }
        finish(dir.path(), kernel).await;
    }

    /// The root's `$subtree` await grant of the old plan is not needed: `spawn` writes the
    /// root's end of the project_lead card's `kernel_state` grant, so a spawned
    /// project_lead waits for an agent with `await_set` and no `$subtree` entry exists.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_spawned_project_lead_awaits_without_a_subtree_grant_on_the_root() {
        let (dir, kernel) = team().await;
        assert!(
            meta(&kernel, ROOT_ID)["callable_by"]
                .get("$subtree")
                .is_none(),
            "the new path writes no `$subtree` entry on the root"
        );
        let pl = call_as(
            &kernel,
            LEAD,
            ROLES,
            json!({"type": "spawn", "role": "project_lead"}),
        )
        .await;
        let pl = pl["id"]
            .as_str()
            .unwrap_or_else(|| panic!("{pl}"))
            .to_string();
        let w = call_as(
            &kernel,
            LEAD,
            ROLES,
            json!({"type": "spawn", "role": "worker"}),
        )
        .await;
        let w = w["id"].as_str().unwrap().to_string();
        let out = call_as(
            &kernel,
            &pl,
            "kernel_state",
            json!({"type": "await_set", "ids": [w], "timeout_s": 5}),
        )
        .await;
        assert!(out.get("error").is_none(), "{out}");
        finish(dir.path(), kernel).await;
    }

    /// The cadencer on the new path: made with `create_agent` (as the examples script
    /// shows), its `group_settings` ride in its record, every edge it needs is written on
    /// both ends, and a beat reaches a spawned project_lead through the real gate.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_cadencer_made_with_create_agent_is_wired_and_delivers_a_digest() {
        let (dir, kernel, sent) = team_with_log().await;
        let edges = json!({
            "board": ["rows"], "roles": ["fleet", "get_count"], "tasks": ["list"],
            "kernel_state": ["reflect"], "lead": ["send"],
        });
        let settings = json!({"lead": {"cadence": 120, "stall_after": 600, "quiet": false,
                                       "enabled": true}});
        let mut can_call = edges.clone();
        can_call["$self"] = json!("*");
        let o = ask(
            &kernel,
            ROOT_ID,
            json!({"type": "create_agent", "id": "cadencer", "parent_id": ROOT_ID,
                   "handler_module": "team_cadencer.tools", "board": "board", "roles": "roles",
                   "group_settings": settings, "can_call": can_call,
                   "callable_by": {"$self": "*", "$parent": "*"}}),
        )
        .await;
        assert!(o.get("error").is_none(), "{o}");
        // the other end of each edge, merged into what the callee has
        for (callee, verbs) in edges.as_object().unwrap() {
            kernel
                .edit_callable_by(&AgentId::from(callee.as_str()), |cb| {
                    cb.insert("cadencer".into(), verbs.clone());
                    true
                })
                .unwrap();
        }
        let cad = meta(&kernel, "cadencer");
        assert_eq!(
            cad["group_settings"], settings,
            "the groups get their settings"
        );
        for (callee, verbs) in edges.as_object().unwrap() {
            assert_eq!(cad["can_call"][callee], *verbs, "{callee}");
            assert_eq!(
                meta(&kernel, callee)["callable_by"]["cadencer"],
                *verbs,
                "{callee} admits the cadencer"
            );
        }
        let st = ask(&kernel, "cadencer", json!({"type": "status"})).await;
        assert_eq!(st["settings"]["enabled"], true, "{st}");
        // a project_lead the main lead spawns, and the worker it spawns: a beat digests the
        // branch. The project_lead card does not grant the worker's tools, which a worker's
        // card does and a spawn grants only what the spawner holds, so the operator gives
        // this project_lead the two tools first.
        let pl = call_as(
            &kernel,
            LEAD,
            ROLES,
            json!({"type": "spawn", "role": "project_lead"}),
        )
        .await;
        let pl = pl["id"]
            .as_str()
            .unwrap_or_else(|| panic!("{pl}"))
            .to_string();
        let have = meta(&kernel, &pl)["can_call"].clone();
        let mut more = have.clone();
        more[WORKER_EXEC] = json!("*");
        more[WORKER_FILES] = json!("*");
        let o = ask(
            &kernel,
            ROOT_ID,
            json!({"type": "update_agent", "id": pl, "can_call": more}),
        )
        .await;
        assert!(o.get("error").is_none(), "{o}");
        let w = call_as(
            &kernel,
            &pl,
            ROLES,
            json!({"type": "spawn", "role": "worker"}),
        )
        .await;
        let w = w["id"]
            .as_str()
            .unwrap_or_else(|| panic!("{w}"))
            .to_string();
        let set = ask(&kernel, "cadencer", json!({"type": "set", "cadence": 1})).await;
        assert!(set.get("error").is_none(), "{set}");
        let mut got = false;
        for _ in 0..40 {
            got = sent.lock().unwrap().iter().any(|(to, p)| {
                to == &pl && p["text"].as_str().is_some_and(|t| t.contains(w.as_str()))
            });
            if got {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        let st = ask(&kernel, "cadencer", json!({"type": "status"})).await;
        assert!(
            got,
            "no digest about {w} reached {pl}: {st} {:?}",
            sent.lock().unwrap()
        );
        assert!(st["digests_sent"].as_u64().unwrap_or(0) >= 1, "{st}");
        finish(dir.path(), kernel).await;
    }
}
