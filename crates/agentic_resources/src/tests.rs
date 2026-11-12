//! Unit tests: every one is driven through `kernel.send`, as production is. A
//! `caller.tools` agent stands for any agent that makes calls (a lead, a worker).

use super::*;
use fantastic_kernel::Agent;
use tempfile::TempDir;

/// An agent that makes the call it is told to (`call {target, payload}`) AS ITSELF,
/// so the gate and `spawn`'s caller see a real agent. Every other verb answers ok.
/// Every `send` a caller agent received: (agent, text).
static SENT: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

/// Every `pause` a caller agent received: the agent id.
static PAUSED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

struct CallerBundle;

fantastic_kernel::stand_in_verbs!(caller_verbs {
    SendV "send" { text: Option<String> },
    HistoryV "history" {},
    PauseV "pause" {},
    ResumeV "resume" {},
    StopV "stop" {},
    ReadV "read" {},
    WriteV "write" {},
});

/// The `Handler` boilerplate of a stand-in: its name, verbs and an open record.
macro_rules! stand_in {
    ($ty:ty, $name:literal, $verbs:ident) => {
        type Call = $verbs::Call;
        fn name(&self) -> &str {
            $name
        }
        fn man(&self) -> &'static str {
            "A test stand-in."
        }
        fn verbs(&self) -> &'static [fantastic_kernel::Verb] {
            $verbs::VERBS
        }
        fn dynamic_verbs(&self, agent: &fantastic_kernel::Agent) -> Vec<fantastic_kernel::DynVerb> {
            $verbs::dynamic_verbs(agent)
        }
        fn record(&self) -> &'static fantastic_kernel::RecordDecl {
            fantastic_kernel::fixture_record!()
        }
    };
}

#[async_trait]
impl Handler for CallerBundle {
    stand_in!(CallerBundle, "caller", caller_verbs);
    async fn handle(
        &self,
        agent_id: &AgentId,
        call: Self::Call,
        _kernel: &Arc<Kernel>,
    ) -> Result<Reply, BundleError> {
        let payload = call.payload();
        let verb = payload.get("type").and_then(Value::as_str).unwrap_or("");
        match verb {
            "reflect" => Ok(Some(
                json!({"id": agent_id.as_str(), "sentence": "a caller"}),
            )),
            "send" => {
                let text = payload["text"].as_str().unwrap_or("").to_string();
                SENT.lock()
                    .unwrap()
                    .push((agent_id.as_str().to_string(), text));
                Ok(Some(json!({"ok": true, "verb": "send"})))
            }
            other => {
                if other == "pause" {
                    PAUSED.lock().unwrap().push(agent_id.as_str().to_string());
                }
                Ok(Some(json!({"ok": true, "verb": other})))
            }
        }
    }
}

/// Stands for ai_core without the dependency: `fork {at}` makes a SIBLING with the
/// source's record minus `callable_by`/`delete_lock`, as ai_core's fork does. `at` 99
/// is refused `cuts_turn`, 98 `no_such_row`.
struct ForkBundle;

fantastic_kernel::stand_in_verbs!(fork_verbs {
    SendV "send" { text: Option<String> },
    HistoryV "history" {},
    PauseV "pause" {},
    ResumeV "resume" {},
    StopV "stop" {},
    ForkV "fork" { at: i64 },
});

#[async_trait]
impl Handler for ForkBundle {
    stand_in!(ForkBundle, "forkable", fork_verbs);
    async fn handle(
        &self,
        agent_id: &AgentId,
        call: Self::Call,
        kernel: &Arc<Kernel>,
    ) -> Result<Reply, BundleError> {
        let payload = &call.payload();
        let verb = payload.get("type").and_then(Value::as_str).unwrap_or("");
        if verb == "send" {
            let text = payload["text"].as_str().unwrap_or("").to_string();
            SENT.lock()
                .unwrap()
                .push((agent_id.as_str().to_string(), text));
            return Ok(Some(json!({"ok": true, "verb": "send"})));
        }
        if verb == "pause" {
            PAUSED.lock().unwrap().push(agent_id.as_str().to_string());
        }
        if verb != "fork" {
            return Ok(Some(json!({"ok": true, "verb": verb})));
        }
        let at = payload["at"].as_i64().unwrap_or(0);
        let refuse = |reason: &str| {
            Ok(Some(json!({
                "error": format!("fork: refused ({reason})"), "reason": reason,
                "hint": "a hint", "at": at,
            })))
        };
        if at == 99 {
            return refuse("cuts_turn");
        }
        if at == 98 {
            return refuse("no_such_row");
        }
        let src = agent_of(kernel, agent_id).unwrap();
        let mut meta = meta_of(&src);
        for k in ["callable_by", "delete_lock"] {
            meta.remove(k);
        }
        meta.insert("forked_from".into(), json!(agent_id.as_str()));
        let root = kernel.root().unwrap();
        let made = fantastic_kernel::lifecycle::create_agent(
            kernel,
            &root,
            fantastic_kernel::declared::kernel::CreateAgent {
                handler_module: src.handler_module.clone().unwrap_or_default(),
                parent_id: src
                    .parent_id
                    .as_ref()
                    .map_or("kernel", |p| p.as_str())
                    .to_string(),
                id: None,
                fields: meta,
            },
        )
        .await;
        let id = made["id"].clone();
        Ok(Some(json!({"id": id, "copied": 3, "at": at})))
    }
}

/// A tool whose record refuses every `callable_by` edit while it carries
/// `refuse_grants` (stands for a bundle whose `validate_meta` says no).
struct PickyBundle;

fantastic_kernel::stand_in_verbs!(picky_verbs { PingV "ping" {} });

#[async_trait]
impl Handler for PickyBundle {
    stand_in!(PickyBundle, "picky", picky_verbs);
    async fn handle(
        &self,
        _agent_id: &AgentId,
        _call: Self::Call,
        _kernel: &Arc<Kernel>,
    ) -> Result<Reply, BundleError> {
        Ok(None)
    }
    fn validate_meta(
        &self,
        meta: &Map<String, Value>,
        record: &Map<String, Value>,
    ) -> Option<String> {
        (meta.contains_key("callable_by") && record.get("refuse_grants") == Some(&json!(true)))
            .then(|| "callable_by refused".to_string())
    }
}

const WRITER: &str = "---\nname: writer\npurpose: Writes the first draft of a text.\nmodel: from-the-card\ngrants:\n  $registry: [bundles]\n  notes: [read]\n---\nYou write clearly and briefly.\n";
const REVIEWER: &str =
    "---\nname: reviewer\npurpose: Reads a draft and says what to fix.\n---\nYou review drafts.\n";
/// fantastic's generic lead card, as a folder card (the registry ships none).
const PROJECT_LEAD: &str = include_str!("../testdata/project_lead.md");
const SUBLEAD: &str = "---\nname: sublead\npurpose: Runs a branch of the work.\ngrants:\n  $registry: [bundles, spawn, get_count]\n  notes: [read]\n---\nYou run one branch.\n";

/// The card texts of these tests (a `---` yaml header: `name`, `purpose`, `grants`; then
/// the directive) loaded as what spawnable bundles are: META BUNDLES over `caller.tools`, their
/// directive and grants in `record`. A text that does not parse is a bundle that failed
/// to load (listed with its error).
fn write_cards(kernel: &Kernel, cards: &[(&str, &str)]) {
    for (file, text) in cards {
        let stem = file.trim_end_matches(".md").to_string();
        let parsed = text
            .strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---\n"));
        let Some((head, body)) = parsed else {
            kernel.bundles.refuse_folder(
                &stem,
                json!({"name": stem, "handler_module": stem, "error": format!("card {file}: missing header")}),
            );
            continue;
        };
        let head = fantastic_kernel::yaml_io::load(head).unwrap();
        let name = head["name"].as_str().unwrap().to_string();
        let mut can_call = head
            .get("grants")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        can_call.insert("$self".into(), json!("*"));
        let mut record = json!({"post_system_prompt": body.trim_end(), "can_call": can_call});
        if let Some(cb) = head.get("callable_by") {
            record["callable_by"] = cb.clone();
        }
        kernel.bundles.register_meta(
            &name,
            json!({"name": name, "handler_module": name, "base": "forkable.tools",
                   "sentence": head["purpose"], "record": record}),
        );
    }
}

fn new_kernel() -> Kernel {
    let mut kernel = Kernel::new();
    kernel.bundles.register(
        fantastic_core::HANDLER_MODULE,
        fantastic_core::KernelStateBundle::default(),
    );
    kernel.bundles.register("caller.tools", CallerBundle);
    kernel.bundles.register("picky.tools", PickyBundle);
    kernel.bundles.register("forkable.tools", ForkBundle);
    kernel
        .bundles
        .register(HANDLER_MODULE, AgenticResourcesBundle);
    kernel
}

fn root_of(kernel: &Kernel, tmp: &TempDir) -> Arc<Agent> {
    let root = Agent::new(
        AgentId::from("kernel_state"),
        Some(fantastic_core::HANDLER_MODULE.to_string()),
        None,
        Map::new(),
        tmp.path().join(".fantastic"),
        false,
    );
    kernel.register(Arc::clone(&root));
    kernel.set_root(Arc::clone(&root));
    root
}

async fn send(kernel: &Arc<Kernel>, target: &str, payload: Value) -> Value {
    kernel
        .send(vec![Item::new(AgentId::from(target), payload)])
        .await
        .remove(0)
        .result
}

async fn create(kernel: &Arc<Kernel>, payload: Value) -> Value {
    let mut p = payload;
    p["type"] = json!("create_agent");
    if p.get("parent_id").is_none() {
        p["parent_id"] = json!("kernel");
    }
    let r = send(kernel, "kernel_state", p).await;
    assert!(r.get("error").is_none(), "create: {r}");
    r
}

/// Per-test ids (the tempdir name is unique): the bundle keeps no state, but ids
/// stay unique across parallel tests anyway.
fn uid(tmp: &TempDir, base: &str) -> String {
    let n = tmp
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .replace('.', "_");
    format!("{base}_{n}")
}

struct Rig {
    kernel: Arc<Kernel>,
    reg: String,
    callers: Vec<String>,
}

/// A kernel with `callers` (caller.tools agents allowed to call the registry) and
/// a registry over `cards`.
async fn rig(tmp: &TempDir, callers: &[&str], cards: &[(&str, &str)]) -> Rig {
    let kernel = Arc::new(new_kernel());
    root_of(&kernel, tmp);
    write_cards(&kernel, cards);
    let reg = uid(tmp, "ar");
    let ids: Vec<String> = callers.iter().map(|c| uid(tmp, c)).collect();
    for c in &ids {
        create(
            &kernel,
            json!({"handler_module": "caller.tools", "id": c, "model": "m1", "provider_id": "prov",
                   "can_call": {reg.clone(): "*", "notes": "*"}}),
        )
        .await;
    }
    let mut cb = Map::new();
    for c in &ids {
        cb.insert(c.clone(), json!("*"));
    }
    create(
        &kernel,
        json!({"handler_module": HANDLER_MODULE, "id": reg, "callable_by": cb}),
    )
    .await;
    Rig {
        kernel,
        reg,
        callers: ids,
    }
}

impl Rig {
    async fn op(&self, payload: Value) -> Value {
        send(&self.kernel, &self.reg, payload).await
    }
    /// `who` calls the registry (or any `target`) as itself.
    async fn call(&self, who: &str, target: &str, payload: Value) -> Value {
        fantastic_kernel::test_support::as_agent(who, send(&self.kernel, target, payload)).await
    }
    async fn spawn_as(&self, who: &str, payload: Value) -> Value {
        let mut p = payload;
        p["type"] = json!("spawn");
        self.call(who, &self.reg, p).await
    }
    fn meta(&self, id: &str) -> Map<String, Value> {
        meta_of(&self.kernel.agents.get(&AgentId::from(id)).unwrap())
    }
    async fn count(&self, bundle: Option<&str>) -> Value {
        match bundle {
            Some(r) => self.op(json!({"type": "get_count", "bundle": r})).await,
            None => self.op(json!({"type": "get_count"})).await,
        }
    }
}

fn id_of(v: &Value) -> String {
    v["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{v}"))
        .to_string()
}

// ── bundles ───────────────────────────────────────────────────────────

#[tokio::test]
async fn bundles_lists_every_card_with_purpose() {
    let tmp = TempDir::new().unwrap();
    let r = rig(
        &tmp,
        &[],
        &[
            ("writer.md", WRITER),
            ("reviewer.md", REVIEWER),
            ("bad.md", "x"),
        ],
    )
    .await;
    let out = r.op(json!({"type": "bundles"})).await;
    let listed = out["bundles"].as_array().unwrap();
    let find = |n: &str| listed.iter().find(|x| x["name"] == n).cloned();
    assert_eq!(
        find("writer").unwrap()["purpose"],
        "Writes the first draft of a text."
    );
    assert_eq!(
        find("reviewer").unwrap()["purpose"],
        "Reads a draft and says what to fix."
    );
    assert!(find("project_lead").is_none(), "no card is built in: {out}");
    assert_eq!(out["errors"], json!(["card bad.md: missing header"]));
    // a bundle loaded later is a bundle added
    write_cards(
        &r.kernel,
        &[(
            "extra.md",
            "---\nname: extra\npurpose: Added later.\n---\nd\n",
        )],
    );
    let again = r.op(json!({"type": "bundles"})).await;
    assert!(again["bundles"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x["name"] == "extra"));
}

#[tokio::test]
async fn bundle_returns_full_card_and_unknown_is_not_found() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &[], &[("writer.md", WRITER)]).await;
    let c = r.op(json!({"type": "bundle", "name": "writer"})).await;
    assert_eq!(c["name"], "writer");
    assert!(
        c.get("tier").is_none(),
        "a bundle names no model and no tier: {c}"
    );
    assert_eq!(c["directive"], "You write clearly and briefly.");
    assert_eq!(c["grants"]["notes"], json!(["read"]));
    let n = r.op(json!({"type": "bundle", "name": "ghost"})).await;
    assert_eq!(n["reason"], "not_found", "{n}");
}

// ── spawn ───────────────────────────────────────────────────────────

#[tokio::test]
async fn spawn_creates_child_with_directive_grants_on_the_spawners_backend() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    let lead = &r.callers[0];
    let out = r
        .spawn_as(lead, json!({"bundle": "writer", "name": "scribe"}))
        .await;
    assert_eq!(out["bundle"], "writer", "{out}");
    assert_eq!(
        out["model"], "m1",
        "the spawner's model when the spawn names none"
    );
    let id = out["id"].as_str().unwrap();
    let a = r.kernel.agents.get(&AgentId::from(id)).unwrap();
    assert_eq!(
        a.parent_id.as_ref().unwrap().as_str(),
        lead.as_str(),
        "a child of the caller"
    );
    let m = meta_of(&a);
    assert_eq!(m["post_system_prompt"], "You write clearly and briefly.");
    assert!(
        m.get("system_prompt").is_none_or(Value::is_null),
        "the default system block stays in force: {m:?}"
    );
    assert_eq!(m["model"], "m1", "the spawner's model; a bundle names none");
    assert_eq!(m["provider_id"], "prov", "the spawner's provider");
    assert!(
        m.get("endpoint").is_none(),
        "no provider field on a chat: {m:?}"
    );
    assert_eq!(
        a.handler_module.as_deref(),
        Some("writer"),
        "an instance of the bundle (on caller.tools)"
    );
    assert!(
        r.kernel.bundles.get("writer").is_some(),
        "which runs its base's code"
    );
    assert_eq!(m["display_name"], "scribe");
    assert_eq!(m["can_call"]["notes"], json!(["read"]));
    assert_eq!(m["can_call"][&r.reg], json!(["bundles"]));
    assert_eq!(
        m["can_call"][lead],
        json!(["send"]),
        "it may tell its parent"
    );
}

#[tokio::test]
async fn spawn_opens_the_spawners_memory_store_to_the_child() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    let mem = uid(&tmp, "mem");
    create(
        &r.kernel,
        json!({"handler_module": "caller.tools", "id": mem}),
    )
    .await;
    // the spawner's store is the child's too
    let mut patch = Map::new();
    patch.insert("memory_store_id".to_string(), json!(mem));
    r.kernel
        .update(&AgentId::from(r.callers[0].as_str()), patch);
    let id = r.spawn_as(&r.callers[0], json!({"bundle": "writer"})).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        r.meta(&id)["can_call"][&mem],
        "*",
        "the child may use its store"
    );
    assert_eq!(
        r.meta(&mem)["callable_by"][&id],
        "*",
        "and the store accepts the child"
    );
}

#[tokio::test]
async fn spawn_runs_the_model_the_spawner_names() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    let out = r
        .spawn_as(&r.callers[0], json!({"bundle": "writer", "model": "m2"}))
        .await;
    assert_eq!(out["model"], "m2", "{out}");
    let id = id_of(&out);
    assert_eq!(r.meta(&id)["model"], "m2");
    assert_eq!(
        r.meta(&id)["provider_id"],
        "prov",
        "on the spawner's provider"
    );
    let bad = r
        .spawn_as(&r.callers[0], json!({"bundle": "writer", "model": 5}))
        .await;
    assert_eq!(bad["reason"], "bad_arg", "{bad}");
}

#[tokio::test]
async fn get_count_after_spawn_delete_restart() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    let lead = &r.callers[0];
    let a = r.spawn_as(lead, json!({"bundle": "writer"})).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    r.spawn_as(lead, json!({"bundle": "writer"})).await;
    assert_eq!(r.count(Some("writer")).await, json!({"running": 2}));
    let gone = send(
        &r.kernel,
        "kernel_state",
        json!({"type": "delete_agent", "id": a}),
    )
    .await;
    assert_eq!(gone["deleted"], true, "{gone}");
    assert_eq!(r.count(Some("writer")).await["running"], 1);
    // a restart: a second kernel loaded from the persisted records
    let state = r.kernel.save();
    let k2 = Arc::new(new_kernel());
    write_cards(
        &k2,
        &[
            ("writer.md", WRITER),
            ("reviewer.md", REVIEWER),
            ("sublead.md", SUBLEAD),
        ],
    ); // the bundles load before the tree
    k2.load(state).unwrap();
    let after = send(&k2, &r.reg, json!({"type": "get_count", "bundle": "writer"})).await;
    assert_eq!(after, json!({"running": 1}), "rebuilt from the records");
    assert_eq!(
        send(&k2, &r.reg, json!({"type": "get_count"})).await["running"],
        1
    );
}

#[tokio::test]
async fn fleet_groups_by_bundle_with_ids() {
    let tmp = TempDir::new().unwrap();
    let r = rig(
        &tmp,
        &["lead"],
        &[("writer.md", WRITER), ("reviewer.md", REVIEWER)],
    )
    .await;
    let lead = &r.callers[0];
    let w1 = r.spawn_as(lead, json!({"bundle": "writer"})).await["id"].clone();
    let w2 = r.spawn_as(lead, json!({"bundle": "writer"})).await["id"].clone();
    let v1 = r.spawn_as(lead, json!({"bundle": "reviewer"})).await["id"].clone();
    create(
        &r.kernel,
        json!({"handler_module": "caller.tools", "id": uid(&tmp, "stray")}),
    )
    .await;
    let f = r.op(json!({"type": "fleet"})).await;
    let ids = |bundle: &str| -> Vec<Value> {
        let mut v: Vec<Value> = f["bundles"][bundle]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["id"].clone())
            .collect();
        v.sort_by_key(ToString::to_string);
        v
    };
    let mut want = vec![w1, w2];
    want.sort_by_key(ToString::to_string);
    assert_eq!(ids("writer"), want, "the stray agent is not listed: {f}");
    assert_eq!(ids("reviewer"), vec![v1]);
    assert_eq!(
        f["bundles"]["writer"][0],
        json!({"id": f["bundles"]["writer"][0]["id"].clone()})
    );
}

#[tokio::test]
async fn a_kernel_with_no_bundles_lists_no_bundles() {
    let tmp = TempDir::new().unwrap();
    let id = uid(&tmp, "ar");
    // the spawnable bundles are the kernel's meta bundles (none here)
    let kernel = Arc::new(new_kernel());
    root_of(&kernel, &tmp);
    create(&kernel, json!({"handler_module": HANDLER_MODULE, "id": id})).await;
    let out = send(&kernel, &id, json!({"type": "bundles"})).await;
    assert_eq!(out, json!({"bundles": [], "errors": []}), "{out}");
}

// ── the extended surface ────────────────────────────────────────────

#[tokio::test]
async fn spawn_grants_parent_pause_resume_stop_both_ends() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead", "sib"], &[("writer.md", REVIEWER)]).await;
    let (lead, sib) = (&r.callers[0], &r.callers[1]);
    let id = r.spawn_as(lead, json!({"bundle": "reviewer"})).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let want = json!(["pause", "resume", "stop"]);
    assert_eq!(r.meta(lead)["can_call"][&id], want);
    assert_eq!(r.meta(&id)["callable_by"][lead], want);
    // nothing wider on the child's side: no other caller is let in by the spawn
    let cb = r.meta(&id)["callable_by"].as_object().unwrap().clone();
    let mut keys: Vec<&String> = cb.keys().collect();
    keys.sort();
    let mut expect = ["$self".to_string(), lead.clone()];
    expect.sort();
    assert_eq!(keys, expect.iter().collect::<Vec<_>>());
    // the lead controls it
    for verb in ["pause", "resume", "stop"] {
        let out = r.call(lead, &id, json!({"type": verb})).await;
        assert_eq!(out["verb"], verb, "{out}");
    }
    // a verb outside the three is refused, and a sibling cannot pause it
    assert_eq!(
        r.call(lead, &id, json!({"type": "send"})).await["reason"],
        "unauthorized"
    );
    assert_eq!(
        r.call(sib, &id, json!({"type": "pause"})).await["reason"],
        "unauthorized"
    );
}

#[tokio::test]
async fn spawn_task_string_is_first_message_and_object_refused() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    let lead = &r.callers[0];
    // a task object (the old task line) is refused
    let before = r.kernel.agents.len();
    let bad = r
        .spawn_as(
            lead,
            json!({"bundle": "writer", "task": {"label": "draft intro"}}),
        )
        .await;
    assert_eq!(bad["reason"], "bad_arg", "{bad}");
    assert_eq!(r.kernel.agents.len(), before, "nothing was created");
    let c = r
        .spawn_as(lead, json!({"bundle": "writer", "task": "do it"}))
        .await;
    assert!(r.meta(c["id"].as_str().unwrap()).get("task").is_none());
    // the string is the child's first message, delivered without holding the spawn
    let cid = c["id"].as_str().unwrap().to_string();
    for _ in 0..100 {
        if SENT.lock().unwrap().iter().any(|(a, t)| {
            *a == cid
                && t.starts_with("do it\n\n[team] Your parent is `")
                && t.contains(&format!("Your parent is `{}`", lead.as_str()))
        }) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(SENT.lock().unwrap().iter().any(|(a, t)| *a == cid
        && t.starts_with("do it\n\n[team] Your parent is `")
        && t.contains(&format!("Your parent is `{}`", lead.as_str()))));
    // the note tells the child to answer its parent with a plain `send`, never a `report` (team#247)
    let note = SENT
        .lock()
        .unwrap()
        .iter()
        .find(|(a, _)| *a == cid)
        .unwrap()
        .1
        .clone();
    assert!(
        note.contains("send it your result when you are done"),
        "{note}"
    );
    assert!(
        note.contains("<call name=\"send\">") && !note.contains("report to it"),
        "{note}"
    );
}

/// Did `who` receive a `send` with `text`? A `send` is answered only once the
/// receiver's handler (its turn) has run, so this is read right after the call.
fn got_send(who: &str, text: &str) -> bool {
    SENT.lock()
        .unwrap()
        .iter()
        .any(|(a, t)| a == who && t == text)
}

#[tokio::test]
async fn spawned_child_sends_its_parent_and_starts_the_parents_turn() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead", "other"], &[("writer.md", WRITER)]).await;
    let (lead, other) = (&r.callers[0], &r.callers[1]);
    let id = id_of(&r.spawn_as(lead, json!({"bundle": "writer"})).await);
    // both ends of the edge child -> parent, `send` only
    assert_eq!(r.meta(&id)["can_call"][lead], json!(["send"]));
    assert_eq!(r.meta(lead)["callable_by"][&id], json!(["send"]));
    // the child's `send` is admitted and runs the parent's handler: the AI parent
    // (a scripted caller.tools agent) gets its turn with the text
    let text = format!("{id}: done, 120 words");
    let out = r
        .call(&id, lead, json!({"type": "send", "text": text}))
        .await;
    assert_eq!(out, json!({"ok": true, "verb": "send"}), "{out}");
    assert!(got_send(lead, &text), "the parent got the message");
    assert!(r.meta(&id).get("task").is_none(), "nothing is stored");
    // only `send`: the child does not control its parent
    assert_eq!(
        r.call(&id, lead, json!({"type": "pause"})).await["reason"],
        "unauthorized"
    );
    // the edge is the child's own: another agent, and the child to a non-parent, are refused
    assert_eq!(
        r.call(other, lead, json!({"type": "send", "text": "x"}))
            .await["reason"],
        "unauthorized"
    );
    assert_eq!(
        r.call(&id, other, json!({"type": "send", "text": "x"}))
            .await["reason"],
        "unauthorized"
    );
}

#[tokio::test]
async fn spawn_from_child_sends_the_spawner() {
    let tmp = TempDir::new().unwrap();
    let (r, src) = fork_rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    let lead = r.callers[0].clone();
    let id = id_of(
        &r.spawn_as(&lead, json!({"from": from(&src, 5)}))
            .await,
    );
    // the clone sits beside its source, which is the spawner's child
    assert_eq!(
        r.kernel
            .agents
            .get(&AgentId::from(id.as_str()))
            .unwrap()
            .parent_id
            .as_ref()
            .map(|p| p.as_str().to_string()),
        Some(lead.clone())
    );
    assert_eq!(r.meta(&id)["can_call"][&lead], json!(["send"]));
    assert_eq!(r.meta(&lead)["callable_by"][&id], json!(["send"]));
    let text = format!("{id}: forked work done");
    let out = r
        .call(&id, &lead, json!({"type": "send", "text": text}))
        .await;
    assert_eq!(out, json!({"ok": true, "verb": "send"}), "{out}");
    assert!(got_send(&lead, &text));
}

#[tokio::test]
async fn child_to_parent_send_is_merged_never_overwritten() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead", "peer"], &[("writer.md", WRITER)]).await;
    let (lead, peer) = (r.callers[0].clone(), r.callers[1].clone());
    // the parent already admits a peer, and a host's reach writes `history` from the
    // child to the parent on both ends before `create_agent` returns
    r.kernel
        .edit_callable_by(&AgentId::from(lead.as_str()), |m| {
            union(m, &Map::from_iter([(String::from(&peer), json!(["send"]))]));
            true
        })
        .unwrap();
    let weak = Arc::downgrade(&r.kernel);
    let l = lead.clone();
    r.kernel
        .add_state_subscriber(Arc::new(move |v: &Value, _body| {
            if v.get("kind").and_then(Value::as_str) != Some("added") {
                return;
            }
            let (Some(id), Some(k)) = (v.get("agent_id").and_then(Value::as_str), weak.upgrade())
            else {
                return;
            };
            if id == l {
                return;
            }
            let (pid, cid) = (l.clone(), id.to_string());
            k.edit_can_call(&AgentId::from(id), move |m| {
                union(
                    m,
                    &Map::from_iter([(String::from(&pid), json!(["history"]))]),
                );
                true
            })
            .unwrap();
            k.edit_callable_by(&AgentId::from(l.as_str()), move |m| {
                union(
                    m,
                    &Map::from_iter([(String::from(&cid), json!(["history"]))]),
                );
                true
            })
            .unwrap();
        }));
    let a = id_of(&r.spawn_as(&lead, json!({"bundle": "writer"})).await);
    let b = id_of(&r.spawn_as(&lead, json!({"bundle": "writer"})).await);
    for id in [&a, &b] {
        assert_eq!(r.meta(id)["can_call"][&lead], json!(["history", "send"]));
        assert_eq!(r.meta(&lead)["callable_by"][id], json!(["history", "send"]));
        // the card's own grants stay
        assert_eq!(r.meta(id)["can_call"]["notes"], json!(["read"]));
    }
    // what the parent held before is kept
    assert_eq!(r.meta(&lead)["callable_by"][&peer], json!(["send"]));
    assert_eq!(r.meta(&lead)["callable_by"]["$self"], json!("*"));
}

#[tokio::test]
async fn operator_spawn_opens_no_edge_to_a_parent() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &[], &[("reviewer.md", REVIEWER)]).await;
    let id = id_of(
        &r.op(json!({"type": "spawn", "bundle": "reviewer", "provider": "prov", "model": "m1"}))
            .await,
    );
    // no caller: the child's parent is the registry, which takes no message; it may
    // call only its provider
    assert_eq!(keys_of(&r, &id, "can_call"), ["$self", "prov"]);
    // and a card that grants nothing on `$registry` opens no edge to it
    assert!(r.meta(&r.reg)["callable_by"].get(&id).is_none());
}

#[tokio::test]
async fn done_frees_nothing_only_the_spawners_delete_agent_does() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    let lead = &r.callers[0];
    let before = r.kernel.agents.len();
    let mut ids = vec![];
    for _ in 0..2 {
        let id = r.spawn_as(lead, json!({"bundle": "writer"})).await["id"]
            .as_str()
            .unwrap()
            .to_string();
        ids.push(id);
    }
    assert_eq!(r.count(None).await["running"], 2);
    // each worker sends `done`: the registry does not delete it, it still counts
    for id in &ids {
        r.call(id, lead, json!({"type": "send", "text": "done"}))
            .await;
    }
    assert_eq!(r.count(None).await["running"], 2, "done frees nothing");
    // the spawner deletes each one: running drops to 0 and the records are as before
    for id in &ids {
        let gone = send(
            &r.kernel,
            "kernel_state",
            json!({"type": "delete_agent", "id": id}),
        )
        .await;
        assert_eq!(gone["deleted"], true, "{gone}");
    }
    assert_eq!(r.count(None).await["running"], 0);
    assert_eq!(r.kernel.agents.len(), before);
}

#[tokio::test]
async fn get_count_counts_every_spawned_agent() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    let lead = &r.callers[0];
    let a = r.spawn_as(lead, json!({"bundle": "writer"})).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    r.spawn_as(lead, json!({"bundle": "writer"})).await;
    assert_eq!(r.count(None).await, json!({"running": 2}));
    // a done message to the parent frees nothing; only delete_agent does
    r.call(&a, lead, json!({"type": "send", "text": "done"}))
        .await;
    assert_eq!(r.count(None).await["running"], 2);
    let gone = send(
        &r.kernel,
        "kernel_state",
        json!({"type": "delete_agent", "id": a}),
    )
    .await;
    assert_eq!(gone["deleted"], true);
    assert_eq!(r.count(None).await["running"], 1);
}

#[tokio::test]
async fn no_auto_cleanup_on_done() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    let lead = &r.callers[0];
    let a = r.spawn_as(lead, json!({"bundle": "writer"})).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let b = r.spawn_as(lead, json!({"bundle": "writer"})).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let before = r.kernel.agents.len();
    for (who, text) in [(&a, "done: ok"), (&b, "failed: no")] {
        let out = r
            .call(who, lead, json!({"type": "send", "text": text}))
            .await;
        assert_eq!(out["verb"], "send", "{out}");
    }
    assert_eq!(r.kernel.agents.len(), before, "nothing was deleted");
    let m = r.meta(&a);
    assert!(m.get("task").is_none());
    assert_eq!(r.count(None).await["running"], 2, "the count stays");
    let f = r.op(json!({"type": "fleet"})).await;
    assert_eq!(
        f["bundles"]["writer"].as_array().unwrap().len(),
        2,
        "still listed: {f}"
    );
    // only an explicit delete removes one
    let gone = send(
        &r.kernel,
        "kernel_state",
        json!({"type": "delete_agent", "id": a}),
    )
    .await;
    assert_eq!(gone["deleted"], true);
    assert_eq!(r.kernel.agents.len(), before - 1);
}

#[tokio::test]
async fn no_card_is_built_in() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["boss"], &[]).await;
    let c = r.op(json!({"type": "bundle", "name": "project_lead"})).await;
    assert_eq!(c["reason"], "not_found", "{c}");
    let out = r
        .spawn_as(&r.callers[0], json!({"bundle": "project_lead"}))
        .await;
    assert!(out.get("id").is_none(), "{out}");
    // a lead card in a folder is a spawnable bundle like any other
    let tmp2 = TempDir::new().unwrap();
    let lead = rig(&tmp2, &["boss"], &[("project_lead.md", PROJECT_LEAD)]).await;
    let c = lead
        .op(json!({"type": "bundle", "name": "project_lead"}))
        .await;
    let grants = c["grants"][REGISTRY_TOKEN]
        .as_array()
        .unwrap_or_else(|| panic!("{c}"))
        .clone();
    for v in ["bundles", "bundle", "spawn", "get_count", "fleet"] {
        assert!(grants.contains(&json!(v)), "grants {v}");
    }
    assert!(lead
        .spawn_as(&lead.callers[0], json!({"bundle": "project_lead"}))
        .await
        .get("id")
        .is_some());
}

// ── spawn from a fork point ─────────────────────────────────────────

/// A rig, plus a source agent that stands
/// for an agent with history: its own prompt, store, grants and stale registry keys.
async fn fork_rig(tmp: &TempDir, callers: &[&str], cards: &[(&str, &str)]) -> (Rig, String) {
    let r = rig(tmp, callers, cards).await;
    let src = uid(tmp, "src");
    // the source runs on the first card's bundle: a clone is of its own bundle
    let bundle = cards[0].0.trim_end_matches(".md");
    create(
        &r.kernel,
        json!({"handler_module": bundle, "id": src, "parent_id": r.callers[0],
               "system_prompt": "SOURCE PROMPT", "model": "src-model",
               "memory_store_id": "src_store",
               "can_call": {"src_store": "*", "secret_agent": "*", r.reg.clone(): ["bundles"]},
               "tier": "old",
               "task": {"label": "old task", "state": "done", "waits_on": [], "result": null}}),
    )
    .await;
    for c in &r.callers {
        grant_fork(&r, c, &src);
    }
    (r, src)
}

fn grant_fork(r: &Rig, who: &str, src: &str) {
    let s = src.to_string();
    r.kernel
        .edit_can_call(&AgentId::from(who), move |m| {
            union(m, &Map::from_iter([(String::from(&s), json!(["fork"]))]));
            true
        })
        .unwrap();
}

fn forks_of(r: &Rig, src: &str) -> Vec<String> {
    r.kernel
        .agents
        .iter()
        .filter(|e| {
            meta_of(e.value())
                .get("forked_from")
                .and_then(Value::as_str)
                == Some(src)
        })
        .map(|e| e.value().id.as_str().to_string())
        .collect()
}

fn from(src: &str, at: i64) -> Value {
    json!({"agent": src, "at": at})
}

#[tokio::test]
async fn spawn_from_makes_a_clone_of_its_source() {
    let tmp = TempDir::new().unwrap();
    let (r, src) = fork_rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    let lead = &r.callers[0];
    let out = r
        .spawn_as(lead, json!({"from": from(&src, 5)}))
        .await;
    assert_eq!(out["bundle"], "writer", "{out}");
    assert_eq!(out["model"], "src-model", "the clone's own model: the source's");
    assert_eq!(out["from"], from(&src, 5));
    assert_eq!(out["copied"], 3);
    let id = out["id"].as_str().unwrap();
    assert_eq!(forks_of(&r, &src), [id], "made by fork");
    // the record is the source's, apart from what makes it another agent
    let rest = |who: &str| {
        let mut m = r.meta(who);
        for k in ["id", "forked_from", "can_call", "callable_by"] {
            m.remove(k);
        }
        m
    };
    assert_eq!(rest(id), rest(&src));
    let m = r.meta(id);
    assert_eq!(m["system_prompt"], "SOURCE PROMPT", "nothing written over it");
    assert_eq!(m["task"]["label"], "old task");
    assert_eq!(m["can_call"]["src_store"], "*");
    assert_eq!(m["can_call"]["secret_agent"], "*");
    // it sits at the source's level
    let level = |who: &str| {
        r.kernel
            .agents
            .get(&AgentId::from(who))
            .unwrap()
            .parent_id
            .clone()
    };
    assert_eq!(level(id), level(&src));
    // the caller controls it and it may tell the caller
    assert_eq!(m["can_call"][lead], json!(["send"]));
    assert_eq!(m["callable_by"][lead], json!(["pause", "resume", "stop"]));
    // the registry admits what the clone holds on it
    assert_eq!(m["can_call"][&r.reg], json!(["bundles"]));
    assert_eq!(r.meta(&r.reg)["callable_by"][id], json!(["bundles"]));
    // the source is untouched
    assert_eq!(r.meta(&src)["task"]["label"], "old task");
}

#[tokio::test]
async fn spawn_from_with_a_bundle_is_refused_by_the_declaration() {
    let tmp = TempDir::new().unwrap();
    let (r, src) = fork_rig(
        &tmp,
        &["lead"],
        &[("writer.md", WRITER), ("reviewer.md", REVIEWER)],
    )
    .await;
    let before = r.meta(&src);
    for bundle in ["writer", "reviewer"] {
        let out = r
            .spawn_as(
                &r.callers[0],
                json!({"bundle": bundle, "from": from(&src, 5)}),
            )
            .await;
        assert_shape(&out, "bad_arg");
        assert!(
            out["error"]
                .as_str()
                .unwrap()
                .contains("only one of bundle | from may be given"),
            "{out}"
        );
    }
    assert!(forks_of(&r, &src).is_empty(), "nothing forked");
    assert_eq!(r.meta(&src), before);
}

#[tokio::test]
async fn spawn_from_task_string_is_sent_as_first_message() {
    let tmp = TempDir::new().unwrap();
    let (r, src) = fork_rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    let text = format!("branch task {}", uid(&tmp, "t"));
    let out = r
        .spawn_as(
            &r.callers[0],
            json!({"from": from(&src, 5), "task": text}),
        )
        .await;
    let id = out["id"].as_str().unwrap().to_string();
    for _ in 0..100 {
        if SENT.lock().unwrap().iter().any(|(a, t)| {
            *a == id
                && t.starts_with(&text)
                && t.contains(&format!("Your parent is `{}`", r.callers[0].as_str()))
        }) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("the child never got its first message");
}

#[tokio::test]
async fn spawn_from_passes_fork_refusals_through() {
    let tmp = TempDir::new().unwrap();
    let (r, src) = fork_rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    let lead = &r.callers[0];
    for (at, reason) in [(99, "cuts_turn"), (98, "no_such_row")] {
        let out = r
            .spawn_as(lead, json!({"from": from(&src, at)}))
            .await;
        assert_eq!(out["reason"], reason, "{out}");
        assert_eq!(out["at"], at);
        assert_eq!(out["hint"], "a hint");
        assert!(out["error"].as_str().unwrap().starts_with("fork:"), "{out}");
        assert!(forks_of(&r, &src).is_empty());
        assert_eq!(
            r.op(json!({"type": "fleet"})).await["bundles"]["writer"]
                .as_array()
                .unwrap()
                .len(),
            1,
            "only the source runs"
        );
        assert_eq!(r.count(None).await["running"], 1);
        let reg = r.meta(&r.reg);
        assert!(reg["can_call"].get(&src).is_none(), "no leftover grant");
        assert!(r.meta(&src)["callable_by"].get(&r.reg).is_none());
    }
}

#[tokio::test]
async fn spawn_from_requires_the_caller_to_hold_fork() {
    let tmp = TempDir::new().unwrap();
    let (r, src) = fork_rig(&tmp, &["lead", "other"], &[("writer.md", WRITER)]).await;
    let (lead, other) = (r.callers[0].clone(), r.callers[1].clone());
    // `other` loses what the rig gave it
    r.kernel
        .edit_can_call(&AgentId::from(other.as_str()), |m| m.remove(&src).is_some())
        .unwrap();
    let out = r
        .spawn_as(&other, json!({"from": from(&src, 5)}))
        .await;
    assert_eq!(out["reason"], "not_allowed", "{out}");
    assert!(
        out["error"].as_str().unwrap().contains("may not fork"),
        "{out}"
    );
    assert!(forks_of(&r, &src).is_empty());
    assert!(r.meta(&r.reg)["can_call"].get(&src).is_none());
    let ok = r
        .spawn_as(&lead, json!({"from": from(&src, 5)}))
        .await;
    assert!(ok.get("id").is_some(), "{ok}");
}

#[tokio::test]
async fn spawn_from_gives_the_parent_pause_resume_stop() {
    let tmp = TempDir::new().unwrap();
    let (r, src) = fork_rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    let lead = &r.callers[0];
    let id = r
        .spawn_as(lead, json!({"from": from(&src, 5)}))
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        r.meta(lead)["can_call"][&id],
        json!(["pause", "resume", "stop"])
    );
    assert_eq!(
        r.meta(&id)["callable_by"][lead],
        json!(["pause", "resume", "stop"])
    );
}

#[tokio::test]
async fn spawn_from_survives_restart_beside_its_source_and_delete_removes_it() {
    let tmp = TempDir::new().unwrap();
    let (r, src) = fork_rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    let lead = &r.callers[0];
    let id = r
        .spawn_as(lead, json!({"from": from(&src, 5)}))
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let state = r.kernel.save();
    let k2 = Arc::new(new_kernel());
    write_cards(
        &k2,
        &[
            ("writer.md", WRITER),
            ("reviewer.md", REVIEWER),
            ("sublead.md", SUBLEAD),
        ],
    ); // the bundles load before the tree
    k2.load(state).unwrap();
    assert_eq!(
        k2.agents
            .get(&AgentId::from(id.as_str()))
            .unwrap()
            .parent_id
            .as_ref()
            .map(|p| p.as_str().to_string()),
        r.kernel
            .agents
            .get(&AgentId::from(src.as_str()))
            .unwrap()
            .parent_id
            .as_ref()
            .map(|p| p.as_str().to_string()),
        "rebuilt from the records, beside its source"
    );
    let gone = send(
        &r.kernel,
        "kernel_state",
        json!({"type": "delete_agent", "id": id}),
    )
    .await;
    assert_eq!(gone["deleted"], true, "{gone}");
    assert!(!r.kernel.agents.contains_key(&AgentId::from(id.as_str())));
}

#[tokio::test]
async fn spawn_from_bad_shape_refused() {
    let tmp = TempDir::new().unwrap();
    let (r, src) = fork_rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    let lead = &r.callers[0];
    for bad_from in [
        json!("x"),
        json!({"at": 1}),
        json!({"agent": src}),
        json!({"agent": src, "at": "1"}),
        json!({"agent": "", "at": 1}),
    ] {
        let out = r
            .spawn_as(lead, json!({"from": bad_from}))
            .await;
        assert!(
            ["bad_arg", "missing_arg"].contains(&out["reason"].as_str().unwrap_or("")),
            "{bad_from}: {out}"
        );
        assert!(
            out["arg"].as_str().unwrap_or("").starts_with("from"),
            "{bad_from}: {out}"
        );
    }
    assert!(forks_of(&r, &src).is_empty());
    // without `from` a spawn is unchanged
    let plain = r.spawn_as(lead, json!({"bundle": "writer"})).await;
    assert!(
        plain.get("id").is_some() && plain.get("from").is_none(),
        "{plain}"
    );
}

// ── spawn merges the parent's grant, never overwrites it (team#245) ──

/// Stands for a host's reach (fantastic's `kern/reach.rs`, #467): when the state stream
/// reports an agent `added`, each of `holders` gets `*` on it and it lets each of them in
/// with `*` — both edges, written in the subscriber, so before `create_agent` returns.
fn reach(r: &Rig, holders: &[String]) {
    let weak = Arc::downgrade(&r.kernel);
    let holders = holders.to_vec();
    r.kernel
        .add_state_subscriber(Arc::new(move |v: &Value, _body| {
            if v.get("kind").and_then(Value::as_str) != Some("added") {
                return;
            }
            let (Some(id), Some(k)) = (v.get("agent_id").and_then(Value::as_str), weak.upgrade())
            else {
                return;
            };
            for h in holders.iter().filter(|h| h.as_str() != id) {
                let new = id.to_string();
                k.edit_can_call(&AgentId::from(h.as_str()), move |m| {
                    union(m, &Map::from_iter([(String::from(&new), json!("*"))]));
                    true
                })
                .unwrap();
                let holder = h.clone();
                k.edit_callable_by(&AgentId::from(id), move |m| {
                    union(m, &Map::from_iter([(String::from(&holder), json!("*"))]));
                    true
                })
                .unwrap();
            }
        }));
}

/// What `lead` may do on `child`, through the kernel's gate.
async fn reaches(r: &Rig, lead: &str, child: &str) {
    for verb in ["send", "history", "pause", "resume", "stop"] {
        let mut call = json!({"type": verb});
        if verb == "send" {
            call["text"] = json!("hi");
        }
        let out = r.call(lead, child, call).await;
        assert!(
            out.get("error").is_none() && out.get("reason").is_none(),
            "{lead} {verb} on {child}: {out}"
        );
    }
}

#[tokio::test]
async fn spawn_under_reach_keeps_the_spawners_star() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead"], &[("reviewer.md", REVIEWER)]).await;
    let lead = r.callers[0].clone();
    reach(&r, std::slice::from_ref(&lead));
    let id = id_of(&r.spawn_as(&lead, json!({"bundle": "reviewer"})).await);
    assert_eq!(r.meta(&lead)["can_call"][&id], json!("*"));
    assert_eq!(r.meta(&id)["callable_by"][&lead], json!("*"));
    reaches(&r, &lead, &id).await;
}

#[tokio::test]
async fn spawn_from_under_reach_keeps_the_spawners_star() {
    let tmp = TempDir::new().unwrap();
    let (r, src) = fork_rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    let lead = r.callers[0].clone();
    reach(&r, std::slice::from_ref(&lead));
    let id = id_of(
        &r.spawn_as(&lead, json!({"from": from(&src, 5)}))
            .await,
    );
    assert_eq!(r.meta(&lead)["can_call"][&id], json!("*"));
    assert_eq!(r.meta(&id)["callable_by"][&lead], json!("*"));
    reaches(&r, &lead, &id).await;
}

#[tokio::test]
async fn spawn_merges_verbs_the_spawner_already_holds() {
    // an edge already there with verbs (not `*`) gains the parent's three, keeps its own
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead"], &[("reviewer.md", REVIEWER)]).await;
    let lead = r.callers[0].clone();
    let weak = Arc::downgrade(&r.kernel);
    let l = lead.clone();
    r.kernel
        .add_state_subscriber(Arc::new(move |v: &Value, _body| {
            if v.get("kind").and_then(Value::as_str) != Some("added") {
                return;
            }
            let (Some(id), Some(k)) = (v.get("agent_id").and_then(Value::as_str), weak.upgrade())
            else {
                return;
            };
            let new = id.to_string();
            k.edit_can_call(&AgentId::from(l.as_str()), move |m| {
                union(
                    m,
                    &Map::from_iter([(String::from(&new), json!(["send", "history"]))]),
                );
                true
            })
            .unwrap();
        }));
    let id = id_of(&r.spawn_as(&lead, json!({"bundle": "reviewer"})).await);
    assert_eq!(
        r.meta(&lead)["can_call"][&id],
        json!(["send", "history", "pause", "resume", "stop"])
    );
}

#[tokio::test]
async fn spawn_without_reach_gives_exactly_the_parent_verbs() {
    let tmp = TempDir::new().unwrap();
    let (r, src) = fork_rig(
        &tmp,
        &["lead"],
        &[("writer.md", WRITER), ("reviewer.md", REVIEWER)],
    )
    .await;
    notes_tool(&r).await;
    let lead = r.callers[0].clone();
    let want = json!(["pause", "resume", "stop"]);
    let plain = id_of(&r.spawn_as(&lead, json!({"bundle": "reviewer"})).await);
    let forked = id_of(
        &r.spawn_as(&lead, json!({"from": from(&src, 5)}))
            .await,
    );
    for id in [&plain, &forked] {
        assert_eq!(r.meta(&lead)["can_call"][id], want, "{id}");
        assert_eq!(r.meta(id)["callable_by"][&lead], want, "{id}");
        assert_eq!(
            r.call(&lead, id, json!({"type": "send", "text": "hi"}))
                .await["reason"],
            "unauthorized",
            "{id}"
        );
    }
}

// ── the card's tool grants (#436) ───────────────────────────────────

const WIDE: &str = "---\nname: wide\npurpose: Reads and writes notes.\ngrants:\n  $registry: [bundles]\n  notes: [read, write]\n---\nYou write.\n";
const STAR: &str =
    "---\nname: star\npurpose: Anything on notes.\ngrants:\n  notes: \"*\"\n---\nYou do all.\n";

async fn notes_tool(r: &Rig) {
    create(
        &r.kernel,
        json!({"handler_module": "caller.tools", "id": "notes"}),
    )
    .await;
}

/// `who` now holds exactly `verbs` on `key` (replacing the rig's grant).
fn hold(r: &Rig, who: &str, key: &str, verbs: Value) {
    let k = key.to_string();
    r.kernel
        .edit_can_call(&AgentId::from(who), move |m| {
            m.insert(k, verbs);
            true
        })
        .unwrap();
}

fn keys_of(r: &Rig, id: &str, field: &str) -> Vec<String> {
    let mut k: Vec<String> = r.meta(id)[field]
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    k.sort();
    k
}

#[tokio::test]
async fn spawn_opens_the_cards_tool_grants_on_both_ends() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    notes_tool(&r).await;
    let before = keys_of(&r, "notes", "callable_by");
    let id = id_of(&r.spawn_as(&r.callers[0], json!({"bundle": "writer"})).await);
    assert_eq!(r.meta(&id)["can_call"]["notes"], json!(["read"]));
    assert_eq!(r.meta("notes")["callable_by"][&id], json!(["read"]));
    let mut want = before;
    want.push(id);
    want.sort();
    assert_eq!(
        keys_of(&r, "notes", "callable_by"),
        want,
        "no other new key"
    );
}

#[tokio::test]
async fn spawned_worker_calls_the_granted_tool_without_directive_setup() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    notes_tool(&r).await;
    let id = id_of(&r.spawn_as(&r.callers[0], json!({"bundle": "writer"})).await);
    let ok = r.call(&id, "notes", json!({"type": "read"})).await;
    assert_eq!(ok["verb"], "read", "{ok}");
    let no = r.call(&id, "notes", json!({"type": "write"})).await;
    assert_eq!(no["reason"], "unauthorized", "{no}");
}

#[tokio::test]
async fn spawn_from_opens_the_tool_grants_on_both_ends() {
    let tmp = TempDir::new().unwrap();
    let (r, src) = fork_rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    notes_tool(&r).await;
    hold(&r, &src, "notes", json!(["read"]));
    let before = keys_of(&r, "notes", "callable_by");
    let id = id_of(
        &r.spawn_as(
            &r.callers[0],
            json!({"from": from(&src, 5)}),
        )
        .await,
    );
    assert_eq!(
        r.meta(&id)["can_call"]["notes"],
        json!(["read"]),
        "the clone holds what its source holds"
    );
    assert_eq!(r.meta("notes")["callable_by"][&id], json!(["read"]));
    let mut want = before;
    want.push(id.clone());
    want.sort();
    assert_eq!(keys_of(&r, "notes", "callable_by"), want);
}

/// A lead bundle whose bundle admits its own subtree's `send` (the scheduler it makes wakes it).
const WAKEABLE: &str = "---\nname: wakeable\npurpose: Runs agents and checks on them.\ncallable_by:\n  $subtree: [send]\n---\nYou check in.\n";

#[tokio::test]
async fn spawn_adds_the_bundles_callable_by_to_the_childs() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead"], &[("wakeable.md", WAKEABLE)]).await;
    let id = id_of(&r.spawn_as(&r.callers[0], json!({"bundle": "wakeable"})).await);
    let cb = &r.meta(&id)["callable_by"];
    assert_eq!(cb["$subtree"], json!(["send"]), "{cb}");
    assert_eq!(
        cb[&r.callers[0]],
        json!(["pause", "resume", "stop"]),
        "the spawner keeps its edge: {cb}"
    );
}

#[tokio::test]
async fn spawn_from_adds_the_bundles_callable_by_to_the_childs() {
    let tmp = TempDir::new().unwrap();
    let (r, src) = fork_rig(&tmp, &["lead"], &[("wakeable.md", WAKEABLE)]).await;
    let id = id_of(
        &r.spawn_as(
            &r.callers[0],
            json!({"from": from(&src, 5)}),
        )
        .await,
    );
    assert_eq!(r.meta(&id)["callable_by"]["$subtree"], json!(["send"]));
}

#[tokio::test]
async fn spawn_tool_grant_with_star_verbs_keeps_star() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead"], &[("star.md", STAR), ("writer.md", WRITER)]).await;
    notes_tool(&r).await;
    let a = id_of(&r.spawn_as(&r.callers[0], json!({"bundle": "star"})).await);
    let b = id_of(&r.spawn_as(&r.callers[0], json!({"bundle": "star"})).await);
    assert_eq!(r.meta("notes")["callable_by"][&a], json!("*"));
    assert_eq!(r.meta("notes")["callable_by"][&b], json!("*"));
    // a grant already there is merged into, never duplicated
    let w = id_of(&r.spawn_as(&r.callers[0], json!({"bundle": "writer"})).await);
    assert_eq!(r.meta("notes")["callable_by"][&w], json!(["read"]));
    assert_eq!(r.meta("notes")["callable_by"][&a], json!("*"));
}

#[tokio::test]
async fn spawn_without_the_tool_agent_still_spawns() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    let id = id_of(&r.spawn_as(&r.callers[0], json!({"bundle": "writer"})).await);
    assert_eq!(r.meta(&id)["can_call"]["notes"], json!(["read"]));
    assert!(r.kernel.agents.get(&AgentId::from("notes")).is_none());
}

#[tokio::test]
async fn refused_tool_grant_undoes_the_spawn() {
    let tmp = TempDir::new().unwrap();
    let picky = uid(&tmp, "zpicky");
    let card = format!(
        "---\nname: two\npurpose: Two tools.\ngrants:\n  $registry: [bundles]\n  notes: [read]\n  {picky}: [read]\n---\nx\n"
    );
    let r = rig(&tmp, &["lead"], &[("two.md", card.as_str())]).await;
    let lead = r.callers[0].clone();
    notes_tool(&r).await;
    create(
        &r.kernel,
        json!({"handler_module": "picky.tools", "id": picky, "refuse_grants": true}),
    )
    .await;
    hold(&r, &lead, &picky, json!("*"));
    let notes_before = r.meta("notes")["callable_by"].clone();
    let reg_before = keys_of(&r, &r.reg, "callable_by");
    let lead_before = keys_of(&r, &lead, "can_call");
    let lead_cb_before = keys_of(&r, &lead, "callable_by");
    let agents_before = r.kernel.agents.len();
    let out = r
        .spawn_as(&lead, json!({"bundle": "two", "task": "go"}))
        .await;
    assert_eq!(out["reason"], "grant_failed", "{out}");
    assert_eq!(r.kernel.agents.len(), agents_before, "no child left");
    assert_eq!(r.op(json!({"type": "fleet"})).await["bundles"], json!({}));
    assert_eq!(r.count(None).await["running"], 0);
    assert_eq!(
        r.meta("notes")["callable_by"],
        notes_before,
        "first edit undone"
    );
    assert_eq!(keys_of(&r, &r.reg, "callable_by"), reg_before);
    assert_eq!(keys_of(&r, &lead, "can_call"), lead_before);
    assert_eq!(
        keys_of(&r, &lead, "callable_by"),
        lead_cb_before,
        "the child -> parent edge is taken back"
    );
}

#[tokio::test]
async fn spawn_refuses_a_grant_the_spawner_does_not_hold() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead"], &[("wide.md", WIDE)]).await;
    notes_tool(&r).await;
    let lead = r.callers[0].clone();
    hold(&r, &lead, "notes", json!(["read"]));
    let notes_before = r.meta("notes")["callable_by"].clone();
    let n = r.kernel.agents.len();
    let out = r.spawn_as(&lead, json!({"bundle": "wide"})).await;
    assert_eq!(out["reason"], "not_allowed", "{out}");
    let why = out["error"].as_str().unwrap();
    assert!(
        why.contains(lead.as_str()) && why.contains("write") && why.contains("notes"),
        "{why}"
    );
    assert_eq!(r.kernel.agents.len(), n, "nothing created");
    assert_eq!(r.meta("notes")["callable_by"], notes_before);
    assert_eq!(r.op(json!({"type": "fleet"})).await["bundles"], json!({}));
}

#[tokio::test]
async fn spawn_refuses_star_grant_when_spawner_holds_only_verbs() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead"], &[("star.md", STAR)]).await;
    notes_tool(&r).await;
    hold(&r, &r.callers[0], "notes", json!(["read"]));
    let out = r.spawn_as(&r.callers[0], json!({"bundle": "star"})).await;
    assert_eq!(out["reason"], "not_allowed", "{out}");
}

#[tokio::test]
async fn spawn_refuses_a_tool_the_spawner_has_no_grant_on() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    notes_tool(&r).await;
    r.kernel
        .edit_can_call(&AgentId::from(r.callers[0].as_str()), |m| {
            m.remove("notes").is_some()
        })
        .unwrap();
    let out = r.spawn_as(&r.callers[0], json!({"bundle": "writer"})).await;
    assert_eq!(out["reason"], "not_allowed", "{out}");
}

#[tokio::test]
async fn spawn_from_refuses_escalation_before_forking() {
    let tmp = TempDir::new().unwrap();
    let (r, src) = fork_rig(&tmp, &["lead"], &[("wide.md", WIDE)]).await;
    notes_tool(&r).await;
    hold(&r, &r.callers[0], "notes", json!(["read"]));
    let out = r
        .spawn_as(
            &r.callers[0],
            json!({"from": from(&src, 5)}),
        )
        .await;
    assert_eq!(out["reason"], "not_allowed", "{out}");
    assert!(forks_of(&r, &src).is_empty(), "nothing forked");
}

#[tokio::test]
async fn operator_spawn_has_no_escalation_check() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &[], &[("wide.md", WIDE)]).await;
    notes_tool(&r).await;
    let id = id_of(
        &r.op(json!({"type": "spawn", "bundle": "wide", "provider": "prov", "model": "m1"}))
            .await,
    );
    assert_eq!(r.meta(&id)["can_call"]["notes"], json!(["read", "write"]));
    assert_eq!(
        r.meta("notes")["callable_by"][&id],
        json!(["read", "write"])
    );
}

#[tokio::test]
async fn registry_token_grants_are_not_escalation_checked() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead"], &[("project_lead.md", PROJECT_LEAD)]).await;
    let lead = r.callers[0].clone();
    hold(&r, &lead, &r.reg, json!(["spawn"]));
    let out = r.spawn_as(&lead, json!({"bundle": "project_lead"})).await;
    assert!(out.get("id").is_some(), "{out}");
}

// ── every existing agent counts (#424) ──────────────────────────────

fn paused_ids() -> Vec<String> {
    PAUSED.lock().unwrap().clone()
}

async fn spawn_id(r: &Rig, who: &str, bundle: &str) -> String {
    let out = r.spawn_as(who, json!({"bundle": bundle})).await;
    out["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{out}"))
        .to_string()
}

#[tokio::test]
async fn get_count_counts_all_existing_agents() {
    let tmp = TempDir::new().unwrap();
    let r = rig(
        &tmp,
        &["lead"],
        &[("writer.md", WRITER), ("reviewer.md", REVIEWER)],
    )
    .await;
    let lead = &r.callers[0];
    let a = spawn_id(&r, lead, "writer").await;
    let b = spawn_id(&r, lead, "writer").await;
    spawn_id(&r, lead, "reviewer").await;
    r.call(lead, &a, json!({"type": "pause"})).await;
    r.call(&b, lead, json!({"type": "send", "text": "done"}))
        .await;
    assert_eq!(r.count(None).await["running"], 3);
    assert_eq!(r.count(Some("writer")).await["running"], 2);
}

#[tokio::test]
async fn done_message_does_not_pause_the_sender() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    let lead = &r.callers[0];
    let a = spawn_id(&r, lead, "writer").await;
    for state in ["done", "failed"] {
        let rep = r
            .call(&a, lead, json!({"type": "send", "text": state}))
            .await;
        assert_eq!(rep, json!({"ok": true, "verb": "send"}), "{rep}");
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        !paused_ids().contains(&a),
        "no pause was sent to the reporter"
    );
    assert_eq!(r.count(None).await["running"], 1);
    let f = r.op(json!({"type": "fleet"})).await;
    assert_eq!(f["bundles"]["writer"].as_array().unwrap().len(), 1, "{f}");
}

#[tokio::test]
async fn the_registry_is_let_in_only_for_what_the_card_grants() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead"], &[("writer.md", WRITER)]).await;
    let lead = &r.callers[0];
    let a = spawn_id(&r, lead, "writer").await;
    // the registry is let in only for what the card grants on it
    assert_eq!(r.meta(&a)["can_call"][&r.reg], json!(["bundles"]));
    assert_eq!(r.meta(&r.reg)["callable_by"][&a], json!(["bundles"]));
}

// ── the refusal shape: `error` is a sentence, `reason` is the code ──────────

/// A refusal in the kernel's shape: `error` a sentence (never the bare code),
/// `reason` the code.
fn assert_shape(out: &Value, code: &str) {
    assert_eq!(out["reason"], code, "{out}");
    let sentence = out["error"].as_str().unwrap_or_else(|| panic!("{out}"));
    assert!(sentence.contains(' ') && sentence != code, "{out}");
}

#[tokio::test]
async fn not_allowed_refusal_reason_is_code() {
    let tmp = TempDir::new().unwrap();
    let r = rig(&tmp, &["lead"], &[("wide.md", WIDE)]).await;
    notes_tool(&r).await;
    let lead = r.callers[0].clone();
    hold(&r, &lead, "notes", json!(["read"]));
    let out = r.spawn_as(&lead, json!({"bundle": "wide"})).await;
    assert_shape(&out, "not_allowed");
    let why = out["error"].as_str().unwrap();
    assert!(
        why.contains(lead.as_str()) && why.contains("write") && why.contains("notes"),
        "{out}"
    );
    assert!(out["hint"].is_string(), "{out}");
}

#[tokio::test]
async fn fork_not_allowed_refusal_reason_is_code() {
    let tmp = TempDir::new().unwrap();
    let (r, src) = fork_rig(&tmp, &["lead", "other"], &[("writer.md", WRITER)]).await;
    let other = r.callers[1].clone();
    r.kernel
        .edit_can_call(&AgentId::from(other.as_str()), |m| m.remove(&src).is_some())
        .unwrap();
    let out = r
        .spawn_as(&other, json!({"from": from(&src, 5)}))
        .await;
    assert_shape(&out, "not_allowed");
    assert!(out["error"].as_str().unwrap().contains("fork"), "{out}");
}

#[tokio::test]
async fn grant_failed_refusal_reason_is_code() {
    let tmp = TempDir::new().unwrap();
    let picky = uid(&tmp, "zpicky");
    let card = format!(
        "---\nname: two\npurpose: Two tools.\ngrants:\n  $registry: [bundles]\n  notes: [read]\n  {picky}: [read]\n---\nx\n"
    );
    let r = rig(&tmp, &["lead"], &[("two.md", card.as_str())]).await;
    let lead = r.callers[0].clone();
    notes_tool(&r).await;
    create(
        &r.kernel,
        json!({"handler_module": "picky.tools", "id": picky, "refuse_grants": true}),
    )
    .await;
    hold(&r, &lead, &picky, json!("*"));
    let before = r.kernel.agents.len();
    let out = r.spawn_as(&lead, json!({"bundle": "two"})).await;
    assert_shape(&out, "grant_failed");
    assert!(
        out["error"].as_str().unwrap().starts_with("spawn:"),
        "{out}"
    );
    assert!(out["hint"].is_string(), "{out}");
    assert_eq!(r.kernel.agents.len(), before, "nothing left behind");
    assert_eq!(r.count(None).await["running"], 0);
}
