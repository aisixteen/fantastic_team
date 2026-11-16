//! Dispatcher: what the team page asks for a node it opens, and the control of a
//! worker or group. `open` grants the page's leg what drawing the node takes (a
//! worker's or subagent's chat, an agent's html view, a file's view agent) and
//! names it; `control` starts, pauses, stops, hands a task to, adds or removes a
//! worker. The tree the page shows is fantastic's sidebar agent, not this one.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::peers::{KernelPeers, Peers};

/// The bundle's handler module name.
pub const HANDLER_MODULE: &str = "team_dispatcher.tools";
/// What the bundle is for: its one short man text; how to call it is rendered from its
/// declared verbs (`docs`, `reflect`).
const MAN: &str = "What the team page opens for a node (it grants the page's leg the node's chat, html view or file view), and the control of a worker or group. No verb assigns work.";

pub mod record;
pub mod verbs;

/// The plan's `fleet` {running} from a registry's `get_count` reply: none when the
/// reply is an error or lacks a count.
pub fn fleet_of(count: &Value) -> Option<Value> {
    if count.get("error").is_some() {
        return None;
    }
    let running = count.get("running")?.as_u64()?;
    Some(json!({"running": running}))
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

/// The `open` reply for an agent whose own `reflect` carries its html view (the task
/// registry, the cadencer, a connector, the bundles registry): the page mounts that view,
/// over `reflect` and the view's calls that `declared_views::grant_views` granted the leg.
pub fn view_open(node: &str) -> Value {
    json!({
        "view": "html",
        "target": node,
        "fullscreen": true,
        "back": "sidebar",
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
pub fn kind_of(node: &str, team: &str, groups: &BTreeMap<String, Vec<String>>) -> &'static str {
    if node == team {
        "team"
    } else if groups.contains_key(node) {
        "group"
    } else {
        "worker"
    }
}

/// The agents of bundles mode the page may chat with, from the plan ([`crate::plan::read`]):
/// the lead, then every node of its tree.
fn plan_ids(plan: &Value) -> Vec<String> {
    fn walk(node: &Value, out: &mut Vec<String>) {
        if let Some(id) = node["id"].as_str().filter(|i| !i.is_empty()) {
            if !out.iter().any(|o| o == id) {
                out.push(id.to_string());
            }
        }
        for k in node["children"].as_array().into_iter().flatten() {
            walk(k, out);
        }
    }
    let mut out: Vec<String> = plan["lead"]
        .as_str()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .into_iter()
        .collect();
    walk(&plan["tree"], &mut out);
    out
}

/// Let the page's legs chat with the lead and its agents: the chat verbs only, never `*`.
fn grant_agents(kernel: &Arc<fantastic_kernel::Kernel>, ids: &[String]) {
    if ids.is_empty() {
        return;
    }
    crate::tree::grant_chat_to(kernel, ids, crate::page::WS_ID);
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
    bundles: Option<&'a str>,
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

/// The tier `name` of the bundles registry `bundles` (the registry's own `tiers` field), as
/// the record fields of an AI agent. `Err` names the tiers the registry has.
fn tier_of(
    kernel: &Arc<fantastic_kernel::Kernel>,
    bundles: Option<&str>,
    name: &str,
) -> Result<serde_json::Map<String, Value>, String> {
    let Some(bundles) = bundles else {
        return Err(
            "team_dispatcher: this dispatcher's record names no `bundles` registry, so \
                    it has no tiers to make a worker from"
                .into(),
        );
    };
    let tiers = kernel
        .agents
        .get(&fantastic_kernel::AgentId::from(bundles))
        .and_then(|e| e.meta.read().expect("meta").get("tiers").cloned())
        .and_then(|t| t.as_object().cloned())
        .ok_or_else(|| format!("team_dispatcher: the registry '{bundles}' has no `tiers`"))?;
    match tiers.get(name).and_then(Value::as_object) {
        Some(t) => Ok(t.clone()),
        None => {
            let names: Vec<&str> = tiers.keys().map(String::as_str).collect();
            Err(format!(
                "team_dispatcher: no tier '{name}' in the registry '{bundles}' (have: {})",
                names.join(", ")
            ))
        }
    }
}

/// `add_worker {node: group, id, template}`: create the worker from the tier `template`
/// of the bundles registry's record (`tiers`, in the dispatcher's `bundles` field), keep
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
    let tier = match tier_of(kernel, at.bundles, &template) {
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

/// The dispatcher bundle: `open`, `control`.
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
        let team = text("team");
        let groups = groups_of(&meta);
        let peers = KernelPeers(kernel.clone());
        let out = match call {
            verbs::Call::Boot(_) => Value::Null,
            verbs::Call::Reflect(_) => json!({"id": agent_id.as_str(), "sentence": MAN}),
            verbs::Call::Open(a) => {
                let node = a.node;
                // An agent whose own `reflect` carries its html view: the page's leg gets
                // `reflect` and the view's calls on it, nothing else (the one view grant, as
                // at boot; an agent made since then gets it here).
                let module = kernel
                    .agents
                    .get(&fantastic_kernel::AgentId::from(node.as_str()))
                    .and_then(|e| e.value().handler_module.clone());
                if module.is_some_and(|m| {
                    crate::declared_views::page_views()
                        .iter()
                        .any(|(h, _)| *h == m)
                }) {
                    crate::declared_views::grant_views(kernel);
                    return Ok(Some(view_open(&node)));
                }
                // The view agent of a file the person opened (made by the files bridge's `open_view`): the
                // leg draws it, with `reflect`, `read` and `watch` on it.
                if crate::tree::is_file_view(kernel, &node) {
                    crate::tree::grant_file_view(kernel, crate::page::WS_ID, &node);
                    return Ok(Some(json!({"kind": "file", "id": node})));
                }
                let kind = kind_of(&node, &team, &groups);
                // A spawned lead or child is chatted with like a worker: grant it now.
                // Only an id the plan's tree names is granted, never an arbitrary one.
                if let Some(bundles) = meta.get("bundles").and_then(Value::as_str) {
                    if kind == "worker" {
                        let ids = plan_ids(&crate::plan::read(&peers, bundles).await);
                        if ids.contains(&node) {
                            grant_agents(kernel, &ids);
                        }
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
                            bundles: meta.get("bundles").and_then(Value::as_str),
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
                            bundles: meta.get("bundles").and_then(Value::as_str),
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

    #[test]
    fn open_classifies_team_group_worker_with_one_helper() {
        let g = groups_a();
        assert_eq!(kind_of("team", "team", &g), "team");
        assert_eq!(kind_of("A", "team", &g), "group");
        assert_eq!(kind_of("w1", "team", &g), "worker");
        assert_eq!(kind_of("nope", "team", &g), "worker");
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

    /// A kernel with a dispatcher (the agent verbs on the root, `bundles` naming the
    /// registry), a bundles registry whose `tiers` field holds the tier `t`,
    /// the stock tree's store `memory`, and one group `g` with `w1`.
    async fn kernel_with_groups(dir: &std::path::Path) -> Booted {
        use fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
        use fantastic_kernel::Item;
        let kernel = boot_at(dir, Seed::Stock, &[], None).await;
        let can_call = json!({ (DEFAULT_ROOT_ID): crate::tree::DISPATCHER_KERNEL_VERBS });
        for item in [
            json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "dispatcher",
                "handler_module": HANDLER_MODULE, "can_call": can_call,
                "bundles": "bundles", "groups": {"g": ["w1"]}}),
            json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "bundles",
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
    async fn control_via_kernel_add_worker_creates_the_agent_and_lists_it_in_its_group() {
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

    /// A plain agent's stand-in (the leg, the lead and its child): answers `get_count`
    /// with {running: 2}.
    struct CountBundle;

    fantastic_kernel::stand_in_verbs!(count_verbs { GetCountV "get_count" { bundle: Option<String> } });

    #[async_trait::async_trait]
    impl fantastic_kernel::bundle::Handler for CountBundle {
        stand_in_body!(count_verbs);

        fn name(&self) -> &str {
            "count_test.tools"
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
                Some("get_count") => Some(json!({"running": 2})),
                _ => None,
            })
        }
    }

    #[test]
    fn fleet_of_reads_a_count_and_ignores_errors() {
        assert_eq!(
            fleet_of(&json!({"running": 1})),
            Some(json!({"running": 1}))
        );
        assert_eq!(fleet_of(&json!({"error": "x", "running": 1})), None);
        assert_eq!(fleet_of(&json!({})), None);
    }

    // ---- the page's chat surface, run in node against a stub DOM ----

    // ---- bundles mode: the agents list and the plan (#185) ----

    /// A registry stand-in for the plan's reads: as the bundles registry it answers `fleet`
    /// with the `fleet` its meta holds and `get_count` with {running: 2}; as the task
    /// registry it answers `list` with the `tasks` its meta holds.
    struct PlanPeer;

    fn register_bundles_mocks(reg: &mut fantastic_kernel::BundleRegistry) {
        reg.register("count_test.tools", CountBundle);
        reg.register("plan_test.tools", PlanPeer);
    }

    fantastic_kernel::stand_in_verbs!(plan_verbs {
        FleetV "fleet" {}, CountV "get_count" {}, ListV "list" {}
    });

    #[async_trait::async_trait]
    impl fantastic_kernel::bundle::Handler for PlanPeer {
        stand_in_body!(plan_verbs);

        fn name(&self) -> &str {
            "plan_test.tools"
        }

        async fn handle(
            &self,
            agent_id: &fantastic_kernel::AgentId,
            call: Self::Call,
            kernel: &Arc<fantastic_kernel::Kernel>,
        ) -> Result<fantastic_kernel::bundle::Reply, fantastic_kernel::bundle::BundleError>
        {
            let m = kernel
                .agents
                .get(agent_id)
                .map(|a| a.meta.read().unwrap().clone())
                .unwrap_or_default();
            let out = match call {
                plan_verbs::Call::FleetV(_) => m.get("fleet").cloned().unwrap_or(json!({})),
                plan_verbs::Call::CountV(_) => json!({"running": 2}),
                plan_verbs::Call::ListV(_) => {
                    json!({"tasks": m.get("tasks").cloned().unwrap_or(json!([]))})
                }
                _ => return Ok(None),
            };
            Ok(Some(out))
        }
    }

    /// A kernel with a dispatcher (bundles mode when `plan` is given: its registry `reg`
    /// lists the project_lead `pl`, the task registry `tasks` holds the plan's tasks, and
    /// the root's `reflect` gives the tree), the `team_ws` leg, and the agents `pl` (the
    /// lead) and `c1` (its child), made after the start so nobody granted them a chat.
    async fn kernel_in_bundles_mode(dir: &std::path::Path, plan: Option<Value>) -> Booted {
        use fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
        use fantastic_kernel::Item;
        let kernel = boot_at(dir, Seed::Stock, &[register_bundles_mocks], None).await;
        let mut d = json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "dispatcher",
            "handler_module": HANDLER_MODULE,
            "can_call": {"reg": "*", "tasks": "*", (DEFAULT_ROOT_ID): ["reflect"]}});
        let mut reg = json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "reg",
            "handler_module": "plan_test.tools", "callable_by": {"dispatcher": "*"}});
        let mut tasks = json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "tasks",
            "handler_module": "plan_test.tools", "callable_by": {"dispatcher": "*"}});
        if let Some(p) = plan {
            d["bundles"] = json!("reg");
            reg["fleet"] = json!({"bundles": {"project_lead": [{"id": p["lead"]}],
                "worker": [{"id": "c1"}]}});
            tasks["tasks"] = p["tasks"].clone();
        }
        let ws = json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "team_ws",
            "handler_module": "count_test.tools"});
        let pl = json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "pl",
            "handler_module": "count_test.tools"});
        let c1 = json!({"type": "create_agent", "parent_id": "pl", "id": "c1",
            "handler_module": "count_test.tools"});
        for item in [d, reg, tasks, ws, pl, c1] {
            let o = kernel.send(vec![Item::new(DEFAULT_ROOT_ID, item)]).await;
            assert!(o[0].result.get("error").is_none(), "{:?}", o[0].result);
        }
        // the root's end of the dispatcher's `reflect` on it (an operator write)
        kernel
            .edit_callable_by(&fantastic_kernel::AgentId::from(DEFAULT_ROOT_ID), |cb| {
                cb.insert("dispatcher".into(), json!(["reflect"]));
                true
            })
            .unwrap();
        kernel
    }

    /// The plan the stand-ins serve: `pl` leads `c1`, which holds `o/r#1`.
    fn lead_section() -> Value {
        json!({"lead": "pl",
            "tasks": [{"id": "o/r#1", "source": "gh:o/r#1", "holder": "c1", "status": "running"}]})
    }

    /// A file row of the tree (#351): the page makes the file's view agent with the files bridge's `open_view`,
    /// then `open` on that id grants the page leg `reflect`, `read` and `watch` on it and nothing else; an agent
    /// that is not a file view gets no such grant.
    #[tokio::test(flavor = "multi_thread")]
    async fn open_of_a_file_view_grants_the_leg_its_reads_and_only_those() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_in_bundles_mode(dir.path(), None).await;
        let made = ask_kernel(
            &kernel,
            "files",
            json!({"type": "open_view", "kind": "file", "path": "a.txt"}),
        )
        .await;
        let vid = made["id"]
            .as_str()
            .unwrap_or_else(|| panic!("{made}"))
            .to_string();
        let out = ask_kernel(&kernel, "dispatcher", json!({"type": "open", "node": vid})).await;
        assert_eq!(out["kind"], "file", "{out}");
        let ws = meta_of(&kernel, "team_ws");
        assert_eq!(
            ws["can_call"][&vid],
            json!(["reflect", "read", "watch"]),
            "{ws:?}"
        );
        let view = meta_of(&kernel, &vid);
        assert_eq!(
            view["callable_by"]["team_ws"],
            json!(["reflect", "read", "watch"])
        );
        // a second `open` changes nothing; an agent that is no file view is not granted
        ask_kernel(&kernel, "dispatcher", json!({"type": "open", "node": vid})).await;
        assert_eq!(
            meta_of(&kernel, "team_ws")["can_call"][&vid],
            json!(["reflect", "read", "watch"])
        );
        ask_kernel(
            &kernel,
            "dispatcher",
            json!({"type": "open", "node": "files"}),
        )
        .await;
        assert!(meta_of(&kernel, "team_ws")["can_call"]
            .get("files")
            .is_none());
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn open_of_a_bundles_agent_returns_the_chat_view_and_grants_only_known_ids() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_in_bundles_mode(dir.path(), Some(lead_section())).await;
        let out = ask_kernel(&kernel, "dispatcher", json!({"type": "open", "node": "pl"})).await;
        assert_eq!(out["view"], "chat", "{out}");
        assert_eq!(out["target"], "pl");
        let ws = meta_of(&kernel, "team_ws");
        assert_eq!(ws["can_call"]["pl"], json!(crate::tree::CHAT_VERBS));
        // An id the bundles tree does not name gets no grant.
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

    #[tokio::test(flavor = "multi_thread")]
    async fn open_of_a_task_registry_answers_its_html_view_and_grants_the_leg_its_reads() {
        use fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
        use fantastic_kernel::Item;
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_in_bundles_mode(dir.path(), None).await;
        // a registry with a store the operator already lets the leg read `list` on
        kernel
            .send(vec![Item::new(
                DEFAULT_ROOT_ID,
                json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "tr",
                    "handler_module": team_task_registry::HANDLER_MODULE,
                    "callable_by": {"team_ws": ["list"], "lead": "*"}}),
            )])
            .await;
        let out = ask_kernel(&kernel, "dispatcher", json!({"type": "open", "node": "tr"})).await;
        assert_eq!(out["view"], "html", "{out}");
        assert_eq!(out["target"], "tr");
        assert_eq!(out["fullscreen"], true);
        let want = json!(["list", "reflect", "watch", "get"]);
        let tasks = meta_of(&kernel, "tr");
        assert_eq!(tasks["callable_by"]["team_ws"], want, "{tasks:?}");
        assert_eq!(tasks["callable_by"]["lead"], "*", "another grant is kept");
        let ws = meta_of(&kernel, "team_ws");
        assert_eq!(
            ws["can_call"]["tr"],
            json!(["reflect", "watch", "list", "get"]),
            "{ws:?}"
        );
        assert!(ws["can_call"].get("*").is_none());
        // the leg now reads the view the registry's reflect carries
        let refl = kernel
            .send(vec![Item::new("tr", json!({"type": "reflect"}))])
            .await
            .remove(0)
            .result;
        assert_eq!(refl["view"]["kind"], "html", "{refl}");
        assert_eq!(
            refl["view"]["calls"],
            json!({"tr": ["watch", "list", "get"]})
        );
        // again: nothing is written twice
        assert_eq!(crate::declared_views::grant_views(&kernel), 0);
        kernel.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn open_of_worker_and_group_unchanged() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = kernel_in_bundles_mode(dir.path(), None).await;
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
        kernel.stop().await;
    }

    #[test]
    fn plan_ids_are_the_lead_then_its_tree_depth_first() {
        let plan = json!({"lead": "pl", "tree": {"id": "pl", "children": [
            {"id": "a", "children": [{"id": "a1", "children": []}]},
            {"id": "b", "children": []}]}});
        assert_eq!(plan_ids(&plan), ["pl", "a", "a1", "b"]);
        assert_eq!(plan_ids(&json!({"lead": "pl", "tree": null})), ["pl"]);
        assert!(plan_ids(&json!({"lead": null, "tree": null})).is_empty());
    }
}
