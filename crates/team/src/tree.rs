//! The agent tree from `team.yaml`: the plan (ids, parents, records), its
//! validation, and its apply to a running kernel. Only an ancestor can write a
//! callee's `callable_by`, so the lead is created under the root and every tool
//! under the lead; the lead then spawns workers itself.
//!
//! Built-in tools (`worker_exec`, `worker_files`, `queue`, `kernel`,
//! `kernel_state`) need no definition in `team.yaml`; anything else a template
//! grants must be a connector defined there.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use fantastic_kernel::bootstrap::{self, DEFAULT_ROOT_ID};
use fantastic_kernel::{Item, Kernel, Status};
use serde_json::{json, Value};

use crate::config::{self, TeamConfig};
use crate::{board, cadencer, dispatcher, pack_bundle, pack_listing, worker_exec, worker_view};

/// The dispatcher's agent id: the team's main view, created at start.
pub const DISPATCHER_ID: &str = "dispatcher";
/// The canvas the dispatcher is placed on at the first start.
pub const CANVAS_ID: &str = "canvas_page";
/// The worker view agent, under the dispatcher; it opens a worker's view.
pub const WORKER_VIEW_ID: &str = "worker_view";
/// The board agent, under the root, when `team.yaml` has a `board` section.
pub const BOARD_ID: &str = "board";
/// The cadencer agent, under the root, when `team.yaml` has a `board` section.
pub const CADENCER_ID: &str = "cadencer";
/// The kernel verbs the dispatcher may call on the root to add and remove a worker
/// (`control` with `add_worker` / `remove_worker`) and to keep the groups lists current.
pub const DISPATCHER_KERNEL_VERBS: [&str; 3] = ["create_agent", "update_agent", "delete_agent"];
/// The page's agent: the dispatcher's html view on the canvas, made at the first start.
pub const DISPATCHER_VIEW_ID: &str = "dispatcher_view";
/// The verbs the page uses on a worker or a subagent: its chat (`history`, `send`,
/// `stop`, `status`) and its events (`watch`). Granted by [`grant_chat`]; never `*`.
pub const CHAT_VERBS: [&str; 5] = ["history", "send", "stop", "status", "watch"];

/// Tools the binary always knows; a template may grant them without a definition.
pub const BUILTIN_TOOLS: [&str; 5] = [
    "worker_exec",
    "worker_files",
    "queue",
    "kernel",
    "kernel_state",
];

/// The pack verbs (`pack`, `install`, `eval`), under the lead: the CLI, MCP and `send` reach them.
pub const PACK_ID: &str = "pack";
/// The pack listing (`list`, `show`, `updates`), under the lead.
pub const PACK_LISTING_ID: &str = "pack_listing";
/// The file bridge the pack agents read and write through, rooted at the team's directory.
pub const FILES_ID: &str = "files";

/// What a planned agent is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The first agent, created under the root.
    Lead,
    /// The dispatcher: the main view, created under the root.
    Dispatcher,
    /// The lead's memory store, under the root.
    Memory,
    /// The credential store, under the root.
    Store,
    /// A tool, under the lead.
    Tool,
    /// The board: the team's cache of its repos, under the root.
    Board,
    /// The cadencer: the digests to the group drivers, under the root.
    Cadencer,
}

/// One agent of the plan: its `create_agent` record and where it goes.
#[derive(Debug, Clone, PartialEq)]
pub struct Planned {
    /// The agent's id.
    pub id: String,
    /// The parent agent's id.
    pub parent: String,
    /// What the agent is for.
    pub role: Role,
    /// The `create_agent` payload.
    pub record: Value,
}

/// What `apply` did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Applied {
    /// Ids created by this apply.
    pub created: Vec<String>,
    /// Ids that already existed and were left as they are.
    pub reused: Vec<String>,
}

/// Refuse a template that grants a tool neither built in nor defined in config.
pub fn check_grants(cfg: &TeamConfig) -> Result<(), String> {
    let mut known: BTreeSet<&str> = BUILTIN_TOOLS.iter().copied().collect();
    known.insert(cfg.queue.id.as_str());
    known.extend(cfg.connectors.keys().map(String::as_str));
    for (name, t) in &cfg.templates {
        let unknown: Vec<&str> = t
            .can_call
            .iter()
            .map(String::as_str)
            .filter(|c| !known.contains(c))
            .collect();
        if !unknown.is_empty() {
            return Err(format!(
                "template `{name}` grants tools not defined in team.yaml: {}",
                unknown.join(", ")
            ));
        }
    }
    Ok(())
}

/// The plan, lead first. Validates the config and the grants.
pub fn plan(cfg: &TeamConfig, team_dir: &Path) -> Result<Vec<Planned>, String> {
    config::validate(cfg)?;
    check_grants(cfg)?;
    let lead = cfg.first_agent.id.clone();
    let tpl = &cfg.templates[&cfg.first_agent.template];
    let mut out = vec![
        Planned {
            id: lead.clone(),
            parent: DEFAULT_ROOT_ID.into(),
            role: Role::Lead,
            record: config::agent_record(tpl, &lead),
        },
        Planned {
            id: config::memory_id(&lead),
            parent: DEFAULT_ROOT_ID.into(),
            role: Role::Memory,
            record: config::memory_record(&lead),
        },
    ];
    if let Some(s) = &cfg.credentials {
        out.push(Planned {
            id: s.id.clone(),
            parent: DEFAULT_ROOT_ID.into(),
            role: Role::Store,
            record: config::store_record(s),
        });
    }
    out.push(Planned {
        id: DISPATCHER_ID.into(),
        parent: DEFAULT_ROOT_ID.into(),
        role: Role::Dispatcher,
        record: json!({
            "type": "create_agent",
            "id": DISPATCHER_ID,
            "parent_id": DEFAULT_ROOT_ID,
            "handler_module": dispatcher::HANDLER_MODULE,
            "board": BOARD_ID,
            "team": lead,
            "groups": dispatcher_groups(cfg),
            // The dispatcher may place itself on the canvas and open worker views;
            // nothing else is granted.
            // It may also ask the cadencer for its status (the cost per hour) and add or
            // remove a worker through the root's agent verbs.
            "can_call": {
                (CANVAS_ID): "*",
                (WORKER_VIEW_ID): "*",
                (CADENCER_ID): ["status"],
                (DEFAULT_ROOT_ID): DISPATCHER_KERNEL_VERBS,
            },
        }),
    });
    // Under the dispatcher, so its `$parent` and `$subtree` baseline lets the
    // dispatcher open a worker's view without a further grant.
    out.push(tool_plan(
        WORKER_VIEW_ID,
        DISPATCHER_ID,
        json!({"handler_module": worker_view::HANDLER_MODULE}),
    ));
    let work = worker_exec::work_dir(team_dir)
        .to_string_lossy()
        .into_owned();
    out.push(tool_plan(
        "worker_exec",
        &lead,
        worker_exec::worker_exec_record(team_dir),
    ));
    out.push(tool_plan(
        "worker_files",
        &lead,
        json!({"handler_module": "file_bridge.tools", "root": work, "allow": ["."]}),
    ));
    // Only the pack agents may reach the files peer; the lead's own grants do not name it.
    // `allow_all_ingress` opens the edge (as the base `store` does); `callable_by` still
    // names who may call.
    let mut files = tool_plan(
        FILES_ID,
        &lead,
        json!({
            "handler_module": "file_bridge.tools",
            "root": team_dir.to_string_lossy().into_owned(),
            "allow": ["."],
            "ingress_rule": "allow_all_ingress",
        }),
    );
    files.record["callable_by"] =
        json!({"$self": "*", "$parent": "*", (PACK_ID): "*", (PACK_LISTING_ID): "*"});
    out.push(files);
    // `eval` and `install --worker` send each eval task to a worker by id, so the pack
    // agent may call the lead and the configured group members (`send` only reaches an
    // agent whose own `callable_by` names `pack`; the lead's does, below).
    let mut pack_can_call = serde_json::Map::new();
    pack_can_call.insert(FILES_ID.into(), json!("*"));
    pack_can_call.insert(lead.clone(), json!("*"));
    // `install --parent-id` builds a pack's blueprint through `user.import_subtree`;
    // the other end of this edge is the `user` agent's `callable_by`, below.
    pack_can_call.insert(USER_ID.into(), json!(PACK_USER_VERBS));
    for m in cfg.groups.values().flat_map(|g| g.members.iter()) {
        pack_can_call.insert(m.clone(), json!("*"));
    }
    out[0].record["callable_by"][PACK_ID] = json!("*");
    out.push(pack_plan(
        PACK_ID,
        &lead,
        json!({"handler_module": pack_bundle::HANDLER_MODULE, "can_call": pack_can_call}),
    ));
    // The `user` agent: its `import_subtree` builds a pack's blueprint. Only the pack
    // agent may call it, and only that verb.
    let mut user = tool_plan(
        USER_ID,
        &lead,
        json!({"handler_module": fantastic_user::HANDLER_MODULE}),
    );
    user.record["callable_by"] = json!({"$self": "*", "$parent": "*", (PACK_ID): PACK_USER_VERBS});
    out.push(user);
    out.push(pack_plan(
        PACK_LISTING_ID,
        &lead,
        json!({
            "handler_module": pack_listing::HANDLER_MODULE,
            "index_path": "packs/index.json",
            "templates_dir": "templates",
            "store": FILES_ID,
            "can_call": {(FILES_ID): "*"},
        }),
    ));
    out.push(tool_plan(
        &cfg.queue.id,
        &lead,
        json!({"handler_module": "ram_store.tools"}),
    ));
    for (id, c) in &cfg.connectors {
        out.push(tool_plan(id, &lead, config::connector_record(id, c)));
    }
    if let Some(b) = &cfg.board {
        // The board is a sibling of the lead, so the GitHub connector and the
        // queue must name it in their callable_by; the lead's own grants stay.
        for p in out
            .iter_mut()
            .filter(|p| p.id == b.github || p.id == cfg.queue.id)
        {
            if let Some(by) = p.record["callable_by"].as_object_mut() {
                by.insert("board".into(), json!("*"));
            }
        }
        out.push(board_plan(cfg, b));
        out.push(cadencer_plan(cfg));
        // The cadencer reads the board's rows and sends the lead its digest.
        if let Some(by) = out
            .iter_mut()
            .find(|p| p.id == BOARD_ID)
            .and_then(|p| p.record["callable_by"].as_object_mut())
        {
            by.insert(CADENCER_ID.into(), json!(["rows"]));
        }
        out[0].record["callable_by"][CADENCER_ID] = json!(["send"]);
    }
    Ok(out)
}

/// The groups as the dispatcher reads them: name → member ids.
pub fn dispatcher_groups(cfg: &TeamConfig) -> Value {
    cfg.groups
        .iter()
        .map(|(n, g)| (n.clone(), json!(g.members)))
        .collect()
}

/// The groups as the board reads them: name → driver ("" for none) and members.
fn team_groups(cfg: &TeamConfig) -> Value {
    cfg.groups
        .iter()
        .map(|(n, g)| {
            let driver = g.driver.clone().unwrap_or_default();
            (n.clone(), json!({"driver": driver, "members": g.members}))
        })
        .collect()
}

/// The board under the root: it reads GitHub and writes its state to the queue.
fn board_plan(cfg: &TeamConfig, b: &config::BoardCfg) -> Planned {
    let mut can_call = serde_json::Map::new();
    can_call.insert(b.github.clone(), json!("*"));
    can_call.insert(cfg.queue.id.clone(), json!("*"));
    Planned {
        id: BOARD_ID.into(),
        parent: DEFAULT_ROOT_ID.into(),
        role: Role::Board,
        record: json!({
            "type": "create_agent",
            "id": BOARD_ID,
            "parent_id": DEFAULT_ROOT_ID,
            "handler_module": board::HANDLER_MODULE,
            "repos": b.repos,
            "github": b.github,
            "store": cfg.queue.id,
            "team_groups": team_groups(cfg),
            "can_call": can_call,
            "callable_by": {"$self": "*", "$parent": "*", (DISPATCHER_ID): "*"},
        }),
    }
}

/// The cadencer under the root: it reads the board and sends each group's digest
/// to its driver. The groups' own `cadence`, `stall_after`, `quiet` and `enabled`
/// from `team.yaml` ride in `group_settings`. It may call the board's `rows`, the
/// `send` of the lead and of every group driver; the dispatcher may ask `status`.
fn cadencer_plan(cfg: &TeamConfig) -> Planned {
    let mut can_call = serde_json::Map::new();
    can_call.insert(BOARD_ID.into(), json!(["rows"]));
    can_call.insert(cfg.first_agent.id.clone(), json!(["send"]));
    for d in cfg.groups.values().filter_map(|g| g.driver.as_ref()) {
        can_call.insert(d.clone(), json!(["send"]));
    }
    Planned {
        id: CADENCER_ID.into(),
        parent: DEFAULT_ROOT_ID.into(),
        role: Role::Cadencer,
        record: json!({
            "type": "create_agent",
            "id": CADENCER_ID,
            "parent_id": DEFAULT_ROOT_ID,
            "handler_module": cadencer::HANDLER_MODULE,
            "board": BOARD_ID,
            "group_settings": config::group_settings(cfg),
            "can_call": can_call,
            "callable_by": {"$self": "*", "$parent": "*", (DISPATCHER_ID): ["status"]},
        }),
    }
}

/// A tool under `parent`: its record with the header a tool needs.
fn tool_plan(id: &str, parent: &str, mut rec: Value) -> Planned {
    if let Some(m) = rec.as_object_mut() {
        m.insert("type".into(), json!("create_agent"));
        m.insert("id".into(), json!(id));
        m.insert("parent_id".into(), json!(parent));
        m.insert("callable_by".into(), json!({"$self": "*", "$parent": "*"}));
    }
    Planned {
        id: id.into(),
        parent: parent.into(),
        role: Role::Tool,
        record: rec,
    }
}

/// A pack agent under `parent`: a tool that the CLI, MCP and `send` may call,
/// and that reads its files through the `files` peer only. Its `callable_by`
/// replaces the tool default, so the dispatcher is named too.
fn pack_plan(id: &str, parent: &str, rec: Value) -> Planned {
    let mut p = tool_plan(id, parent, rec);
    p.record["callable_by"] = json!({"$self": "*", "$parent": "*", "dispatcher": "*"});
    p
}

/// True when the agent `id` is live in the kernel. Read from the kernel's table,
/// not by a `reflect`: a team bundle need not answer `reflect` (the dispatcher
/// and worker view do not), so a reflect would call a live agent missing.
fn exists(kernel: &Arc<Kernel>, id: &str) -> bool {
    kernel
        .agents
        .contains_key(&fantastic_kernel::AgentId::from(id))
}

/// Create the planned agents that are missing; leave the existing ones as they are.
pub async fn apply(kernel: &Arc<Kernel>, plan: &[Planned]) -> Result<Applied, String> {
    let mut out = Applied::default();
    for p in plan {
        if exists(kernel, &p.id) {
            out.reused.push(p.id.clone());
            continue;
        }
        let mut outs = kernel
            .send(vec![Item::new(DEFAULT_ROOT_ID, p.record.clone())])
            .await;
        match outs.pop() {
            Some(o) if matches!(o.status, Status::Completed) => out.created.push(p.id.clone()),
            Some(o) => return Err(format!("creating `{}` failed: {}", p.id, o.result)),
            None => return Err(format!("creating `{}`: no reply", p.id)),
        }
    }
    kernel.flush_records().await;
    Ok(out)
}

/// Whether the first message is due: when the lead was just created, or when asked.
pub fn instructions_due(applied: &Applied, lead: &str, resend: bool) -> bool {
    resend || applied.created.iter().any(|c| c == lead)
}

/// Send the rendered instructions to the first agent as its first message.
pub async fn deliver_instructions(
    kernel: &Arc<Kernel>,
    lead: &str,
    text: &str,
) -> Result<(), String> {
    let mut outs = kernel
        .send(vec![Item::new(lead, json!({"type": "send", "text": text}))])
        .await;
    match outs.pop() {
        Some(o) if matches!(o.status, Status::Completed) => Ok(()),
        Some(o) => Err(format!(
            "delivering instructions to `{lead}` failed: {}",
            o.result
        )),
        None => Err(format!("delivering instructions to `{lead}`: no reply")),
    }
}

/// Put the dispatcher's view on the canvas: the first start only, since the
/// dispatcher is created only then. The team (not the dispatcher) creates the
/// view under the canvas, so the canvas needs no grant to the dispatcher. Best
/// effort: a canvas that is not there is reported, not fatal, so the team still starts.
async fn place_dispatcher(kernel: &Arc<Kernel>) {
    let view = DISPATCHER_VIEW_ID;
    if exists(kernel, view) {
        return;
    }
    let mut outs = kernel
        .send(vec![Item::new(
            DEFAULT_ROOT_ID,
            json!({
                "type": "create_agent",
                "id": view,
                "parent_id": CANVAS_ID,
                "handler_module": "html.tools",
                "html": dispatcher::HTML,
            }),
        )])
        .await;
    if let Some(o) = outs.pop() {
        if o.result.get("error").is_some() || !matches!(o.status, Status::Completed) {
            eprintln!("dispatcher not placed on `{CANVAS_ID}`: {}", o.result);
        }
    }
}

/// The `user` agent (`user.tools`): it holds the blueprint verbs.
pub const USER_ID: &str = "user";
/// The one `user` verb the pack agent may call (`install --parent-id`).
pub const PACK_USER_VERBS: [&str; 1] = ["import_subtree"];

/// Grant the page's agent the chat verbs on each worker in `workers` and on every
/// agent whose parent is one of them (its subagents), on both ends: the view's
/// `can_call` names the agent and the agent's `callable_by` names the view. An
/// operator write made at each start, so it is idempotent. An agent that is not
/// live is skipped; a refusal is reported, not fatal. Returns how many agents got
/// the grant. An agent made after the start gets it at the next start.
pub fn grant_chat(kernel: &Arc<Kernel>, workers: &[String]) -> usize {
    grant_chat_to(kernel, workers, DISPATCHER_VIEW_ID)
}

/// [`grant_chat`] for any principal that carries the page: the canvas view, or the
/// `web_ws` leg that serves the page on a port.
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

/// Name the dispatcher in the root's `callable_by` with the agent verbs
/// ([`DISPATCHER_KERNEL_VERBS`]), the other end of the dispatcher's `can_call` on
/// the root. An operator write made at each start, so it is idempotent. Only
/// when the dispatcher is live.
pub fn grant_agent_verbs(kernel: &Arc<Kernel>) -> Result<(), String> {
    if !exists(kernel, DISPATCHER_ID) {
        return Ok(());
    }
    let verbs = json!(DISPATCHER_KERNEL_VERBS);
    let root = fantastic_kernel::AgentId::from(DEFAULT_ROOT_ID);
    kernel
        .edit_callable_by(&root, |cb| {
            if cb.get(DISPATCHER_ID) == Some(&verbs) {
                return false;
            }
            cb.insert(DISPATCHER_ID.into(), verbs.clone());
            true
        })
        .map(|_| ())
        .map_err(|e| format!("dispatcher agent verbs: {e}"))
}

/// Start-up: plan the tree from `team.yaml` in `dir` (if there is one), apply it
/// to a headless kernel, deliver the instructions when due, then stop the kernel.
/// Returns `None` when there is no `team.yaml`.
pub fn run_at_start(
    dir: &Path,
    instructions: Option<&str>,
    resend: bool,
) -> Result<Option<Applied>, String> {
    let path = dir.join(config::FILE_NAME);
    if !path.exists() {
        return Ok(None);
    }
    let cfg = config::load(&path)?;
    let tree = plan(&cfg, dir)?;
    let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    rt.block_on(async {
        let kernel = crate::start(dir).await.map_err(|e| e.to_string())?;
        let res: Result<Applied, String> = async {
            let applied = apply(&kernel, &tree).await?;
            if applied.created.iter().any(|c| c == DISPATCHER_ID) {
                place_dispatcher(&kernel).await;
            }
            let workers: Vec<String> = cfg
                .groups
                .values()
                .flat_map(|g| g.members.iter().cloned())
                .collect();
            grant_chat(&kernel, &workers);
            if let Err(e) = grant_agent_verbs(&kernel) {
                eprintln!("{e}");
            }
            if let Some(text) = instructions {
                if instructions_due(&applied, &cfg.first_agent.id, resend) {
                    deliver_instructions(&kernel, &cfg.first_agent.id, text).await?;
                }
            }
            Ok(applied)
        }
        .await;
        kernel.shutdown().await;
        bootstrap::shutdown(dir).map_err(|e| e.to_string())?;
        res.map(Some)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse;
    use crate::dispatcher;

    /// The sample config. `{CLONE}` and `{WT}` are filled with absolute paths
    /// from this OS (the git connector requires absolute dirs), inside YAML
    /// single quotes so backslashes survive on Windows.
    const SAMPLE: &str = "templates:\n  lead_t:\n    handler_module: ram_store.tools\n    model: m\n    endpoint: http://e\n    can_call: [worker_exec, worker_files, queue, git]\n  worker_t:\n    handler_module: ram_store.tools\n    model: m\n    endpoint: http://e\n    can_call: [worker_exec, worker_files]\nfirst_agent:\n  template: lead_t\nconnectors:\n  git:\n    kind: team_git.tools\n    record:\n      remote: origin\n      repo_dir: '{CLONE}'\n      workspace_root: '{WT}'\n      default_branch: main\nqueue:\n  id: queue\n";

    fn sample_yaml() -> String {
        let quoted = |p: std::path::PathBuf| p.to_string_lossy().replace('\'', "''");
        let tmp = std::env::temp_dir();
        SAMPLE
            .replace("{CLONE}", &quoted(tmp.join("clone")))
            .replace("{WT}", &quoted(tmp.join("wt")))
    }

    fn sample() -> TeamConfig {
        parse(&sample_yaml()).unwrap()
    }

    #[test]
    fn sample_git_dirs_are_absolute_on_this_os() {
        let dir = tempfile::TempDir::new().unwrap();
        let tree = plan(&sample(), dir.path()).unwrap();
        let git = tree
            .iter()
            .find(|p| p.id == "git")
            .expect("a git connector");
        for key in ["repo_dir", "workspace_root"] {
            let v = git.record[key].as_str().expect(key);
            assert!(Path::new(v).is_absolute(), "{key} = {v}");
        }
    }

    fn ids(t: &[Planned]) -> Vec<String> {
        t.iter().map(|p| p.id.clone()).collect()
    }

    #[test]
    fn sample_config_plan_ids_parents_grants() {
        let tree = plan(&sample(), Path::new("/team")).unwrap();
        assert_eq!(
            ids(&tree),
            [
                "lead",
                "lead_memory",
                "dispatcher",
                "worker_view",
                "worker_exec",
                "worker_files",
                "files",
                "pack",
                "user",
                "pack_listing",
                "queue",
                "git"
            ]
        );
        assert_eq!(tree[0].parent, DEFAULT_ROOT_ID);
        assert_eq!(tree[1].parent, DEFAULT_ROOT_ID);
        for p in tree
            .iter()
            .filter(|p| p.role == Role::Tool && p.id != WORKER_VIEW_ID)
        {
            assert_eq!(p.parent, "lead", "{}", p.id);
        }
        let d = tree.iter().find(|p| p.id == "dispatcher").unwrap();
        assert_eq!(d.parent, DEFAULT_ROOT_ID);
        let v = tree.iter().find(|p| p.id == WORKER_VIEW_ID).unwrap();
        assert_eq!(v.parent, "dispatcher");
        let grants = tree[0].record["can_call"].as_object().unwrap();
        for g in ["worker_exec", "worker_files", "queue", "git"] {
            assert!(grants.contains_key(g), "{g}");
        }
    }

    #[test]
    fn plan_contains_a_dispatcher_agent_with_board_and_team() {
        let tree = plan(&sample(), Path::new("/team")).unwrap();
        let d = tree
            .iter()
            .find(|p| p.id == "dispatcher")
            .expect("the plan has a dispatcher");
        assert_eq!(d.role, Role::Dispatcher);
        assert_eq!(d.parent, DEFAULT_ROOT_ID);
        assert_eq!(d.record["type"], "create_agent");
        assert_eq!(d.record["handler_module"], dispatcher::HANDLER_MODULE);
        assert_eq!(d.record["board"], "board");
        assert_eq!(d.record["team"], "lead");
        assert_eq!(d.record["groups"], json!({}));
    }

    #[test]
    fn lead_is_created_first() {
        let tree = plan(&sample(), Path::new("/team")).unwrap();
        assert_eq!(tree[0].id, "lead");
        assert_eq!(tree[0].role, Role::Lead);
    }

    #[test]
    fn instructions_sent_only_on_first_create_or_when_asked() {
        let created = Applied {
            created: vec!["lead".into()],
            reused: vec![],
        };
        let reused = Applied {
            created: vec![],
            reused: vec!["lead".into()],
        };
        assert!(instructions_due(&created, "lead", false));
        assert!(!instructions_due(&reused, "lead", false));
        assert!(instructions_due(&reused, "lead", true));
    }

    #[test]
    fn template_with_undefined_tool_refused_at_start() {
        let mut cfg = sample();
        cfg.templates
            .get_mut("worker_t")
            .unwrap()
            .can_call
            .push("mystery".into());
        let e = plan(&cfg, Path::new("/team")).unwrap_err();
        assert!(e.contains("worker_t"), "{e}");
        assert!(e.contains("mystery"), "{e}");
    }

    #[test]
    fn unknown_tool_name_is_still_refused() {
        let text = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/team.yaml"),
        )
        .unwrap()
        .replace(
            "can_call: [worker_exec, worker_files, queue]",
            "can_call: [worker_exec, worker_files, queue, ghost]",
        );
        let e = plan(&parse(&text).unwrap(), Path::new("/team")).unwrap_err();
        assert!(e.contains("ghost"), "{e}");
    }

    #[test]
    fn example_team_yaml_validates() {
        let cfg =
            config::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/team.yaml"))
                .unwrap();
        let tree = plan(&cfg, Path::new("/team")).unwrap();
        assert_eq!(tree[0].id, "lead");
    }

    #[test]
    fn plan_contains_no_secret_values() {
        let cfg =
            config::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/team.yaml"))
                .unwrap();
        let tree = plan(&cfg, Path::new("/team")).unwrap();
        let text =
            serde_json::to_string(&tree.iter().map(|p| &p.record).collect::<Vec<_>>()).unwrap();
        assert!(!text.contains("sk-"));
        assert!(!text.contains("Bearer "));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn apply_twice_is_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let tree = plan(&sample(), dir.path()).unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        let first = apply(&kernel, &tree).await.unwrap();
        assert_eq!(first.created, ids(&tree));
        assert!(first.created.iter().any(|c| c == "dispatcher"));
        let second = apply(&kernel, &tree).await.unwrap();
        assert!(second.created.is_empty());
        assert_eq!(second.reused, ids(&tree));
        assert!(second.reused.iter().any(|c| c == "dispatcher"));
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[test]
    fn plan_creates_pack_agents_with_files_peer() {
        let tree = plan(&sample(), Path::new("/team")).unwrap();
        let find = |id: &str| {
            tree.iter()
                .find(|p| p.id == id)
                .unwrap_or_else(|| panic!("the plan has no `{id}`"))
        };
        let files = find(FILES_ID);
        assert_eq!(files.role, Role::Tool);
        assert_eq!(files.parent, "lead");
        assert_eq!(files.record["handler_module"], "file_bridge.tools");
        assert_eq!(files.record["root"], "/team");
        assert_eq!(files.record["allow"], json!(["."]));
        assert_eq!(
            files.record["callable_by"],
            json!({"$self": "*", "$parent": "*", (PACK_ID): "*", (PACK_LISTING_ID): "*"}),
        );

        let pack = find(PACK_ID);
        assert_eq!(pack.parent, "lead");
        assert_eq!(pack.record["handler_module"], pack_bundle::HANDLER_MODULE);
        assert_eq!(
            pack.record["can_call"],
            json!({"files": "*", "lead": "*", "user": ["import_subtree"]})
        );
        assert_eq!(
            pack.record["callable_by"],
            json!({"$self": "*", "$parent": "*", "dispatcher": "*"}),
        );

        let listing = find(PACK_LISTING_ID);
        assert_eq!(listing.parent, "lead");
        assert_eq!(
            listing.record["handler_module"],
            pack_listing::HANDLER_MODULE
        );
        assert_eq!(listing.record["index_path"], "packs/index.json");
        assert_eq!(listing.record["templates_dir"], "templates");
        assert_eq!(listing.record["store"], "files");
        assert_eq!(listing.record["can_call"], json!({"files": "*"}));
        assert!(
            listing.record.get("trusted_keys").is_none(),
            "nothing is trusted until team.yaml says so"
        );
        assert_eq!(
            listing.record["callable_by"],
            json!({"$self": "*", "$parent": "*", "dispatcher": "*"}),
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn applied_pack_listing_answers_list_on_empty_index() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("templates")).unwrap();
        std::fs::create_dir_all(dir.path().join("packs")).unwrap();
        std::fs::write(dir.path().join("packs/index.json"), r#"{"entries":[]}"#).unwrap();
        let tree = plan(&sample(), dir.path()).unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        apply(&kernel, &tree).await.unwrap();
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
        let tree = plan(&sample(), dir.path()).unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        apply(&kernel, &tree).await.unwrap();
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
        let tree = plan(&sample(), dir.path()).unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        apply(&kernel, &tree).await.unwrap();
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

    #[tokio::test(flavor = "multi_thread")]
    async fn applied_pack_eval_reaches_a_worker() {
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
        let cfg = parse(&with_groups_and_board()).unwrap();
        let tree = plan(&cfg, dir.path()).unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        apply(&kernel, &tree).await.unwrap();
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

    #[test]
    fn pack_may_call_user() {
        let cfg = parse(&sample_yaml()).unwrap();
        let tree = plan(&cfg, Path::new("/team")).unwrap();
        let pack = tree.iter().find(|p| p.id == PACK_ID).unwrap();
        assert_eq!(pack.record["can_call"]["user"], json!(["import_subtree"]));
        let user = tree.iter().find(|p| p.id == USER_ID).unwrap();
        assert_eq!(user.record["handler_module"], "user.tools");
        assert_eq!(
            user.record["callable_by"]["pack"],
            json!(["import_subtree"])
        );
    }

    #[test]
    fn plan_lets_pack_call_the_lead_and_group_members() {
        let cfg = parse(&with_groups_and_board()).unwrap();
        let tree = plan(&cfg, Path::new("/team")).unwrap();
        let pack = tree.iter().find(|p| p.id == PACK_ID).unwrap();
        let can = pack.record["can_call"].as_object().unwrap();
        for id in ["files", "lead", "w1", "w2", "w3"] {
            assert_eq!(can.get(id), Some(&json!("*")), "pack cannot call {id}");
        }
        assert_eq!(can.get("user"), Some(&json!(["import_subtree"])));
        let lead = tree.iter().find(|p| p.id == "lead").unwrap();
        assert_eq!(lead.record["callable_by"][PACK_ID], "*");
    }

    /// The sample with groups and a board section added at the end.
    fn with_groups_and_board() -> String {
        sample_yaml()
            + "groups:\n  a:\n    driver: lead\n    members: [w1, w2]\n  b:\n    members: [w3]\nboard:\n  repos: [o/r]\n  github: git\n"
    }

    #[test]
    fn plan_fills_dispatcher_groups_from_config() {
        let cfg = parse(&with_groups_and_board()).unwrap();
        let tree = plan(&cfg, Path::new("/team")).unwrap();
        let d = tree.iter().find(|p| p.id == "dispatcher").unwrap();
        assert_eq!(d.record["groups"], json!({"a": ["w1", "w2"], "b": ["w3"]}));
    }

    #[test]
    fn plan_creates_board_agent_with_groups_and_reachable_github() {
        let tree = plan(
            &parse(&with_groups_and_board()).unwrap(),
            Path::new("/team"),
        )
        .unwrap();
        let b = tree
            .iter()
            .find(|p| p.id == "board")
            .expect("a board agent");
        assert_eq!(b.role, Role::Board);
        assert_eq!(b.parent, DEFAULT_ROOT_ID);
        assert_eq!(b.record["type"], "create_agent");
        assert_eq!(b.record["handler_module"], board::HANDLER_MODULE);
        assert_eq!(b.record["store"], "queue");
        assert_eq!(b.record["github"], "git");
        assert_eq!(b.record["repos"], json!(["o/r"]));
        assert_eq!(b.record["team_groups"]["a"]["driver"], "lead");
        assert_eq!(b.record["team_groups"]["a"]["members"], json!(["w1", "w2"]));
        assert_eq!(b.record["team_groups"]["b"]["driver"], "");
        assert_eq!(b.record["can_call"], json!({"git": "*", "queue": "*"}));
        let git = tree.iter().position(|p| p.id == "git").unwrap();
        let board_at = tree.iter().position(|p| p.id == "board").unwrap();
        assert!(board_at > git, "the board comes after the tools it calls");
        let git_rec = &tree[git].record;
        assert_eq!(git_rec["callable_by"]["board"], "*");
        assert_eq!(git_rec["callable_by"]["$parent"], "*");
    }

    #[test]
    fn plan_creates_a_cadencer_carrying_the_group_settings() {
        let y = sample_yaml()
            + "groups:\n  a:\n    driver: lead\n    members: [w1]\n    cadence: 600\n    quiet: false\n  b:\n    members: [w3]\nboard:\n  repos: [o/r]\n  github: git\n";
        let tree = plan(&parse(&y).unwrap(), Path::new("/team")).unwrap();
        let c = tree
            .iter()
            .find(|p| p.id == CADENCER_ID)
            .expect("a cadencer agent");
        assert_eq!(c.role, Role::Cadencer);
        assert_eq!(c.parent, DEFAULT_ROOT_ID);
        assert_eq!(c.record["handler_module"], cadencer::HANDLER_MODULE);
        assert_eq!(c.record["board"], "board");
        assert_eq!(
            c.record["group_settings"],
            json!({"a": {"cadence": 600, "quiet": false}})
        );
        assert_eq!(c.record["can_call"]["board"], json!(["rows"]));
        assert_eq!(c.record["can_call"]["lead"], json!(["send"]));
        assert_eq!(c.record["callable_by"]["dispatcher"], json!(["status"]));
        // The other ends of the edges name the cadencer.
        let by = |id: &str| tree.iter().find(|p| p.id == id).unwrap().record["callable_by"].clone();
        assert_eq!(by("board")["cadencer"], json!(["rows"]));
        assert_eq!(by("lead")["cadencer"], json!(["send"]));
        // No board section, no cadencer.
        let plain = plan(&sample(), Path::new("/team")).unwrap();
        assert!(!plain.iter().any(|p| p.id == CADENCER_ID));
    }

    #[test]
    fn dispatcher_may_ask_the_cadencer_and_call_the_kernel_agent_verbs() {
        let tree = plan(&sample(), Path::new("/team")).unwrap();
        let d = tree.iter().find(|p| p.id == "dispatcher").unwrap();
        assert_eq!(d.record["can_call"]["cadencer"], json!(["status"]));
        assert_eq!(
            d.record["can_call"][DEFAULT_ROOT_ID],
            json!(["create_agent", "update_agent", "delete_agent"])
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn grant_agent_verbs_lets_the_dispatcher_call_the_kernel() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        let mk = json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "dispatcher",
            "handler_module": dispatcher::HANDLER_MODULE,
            "can_call": {(DEFAULT_ROOT_ID): DISPATCHER_KERNEL_VERBS}});
        kernel.send(vec![Item::new(DEFAULT_ROOT_ID, mk)]).await;
        let create = |id: &str| json!({"type": "control", "node": "g", "action": "remove_worker", "confirm": true, "x": id});
        // Before the grant the root refuses the dispatcher's delete.
        let before = kernel
            .send(vec![Item::new("dispatcher", create("a"))])
            .await;
        assert!(
            before[0].result["error"]
                .as_str()
                .unwrap()
                .contains("does not accept"),
            "{}",
            before[0].result
        );
        grant_agent_verbs(&kernel).unwrap();
        grant_agent_verbs(&kernel).unwrap();
        let after = kernel
            .send(vec![Item::new("dispatcher", create("a"))])
            .await;
        assert!(
            !after[0].result["error"]
                .as_str()
                .unwrap_or("")
                .contains("does not accept"),
            "{}",
            after[0].result
        );
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[test]
    fn plan_without_board_section_has_no_board_agent() {
        let tree = plan(&sample(), Path::new("/team")).unwrap();
        assert!(!tree.iter().any(|p| p.id == "board"));
        let d = tree.iter().find(|p| p.id == "dispatcher").unwrap();
        assert_eq!(d.record["groups"], json!({}));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn place_dispatcher_puts_view_on_the_canvas() {
        let dir = tempfile::TempDir::new().unwrap();
        let tree = plan(&sample(), dir.path()).unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        apply(&kernel, &tree).await.unwrap();
        if !exists(&kernel, CANVAS_ID) {
            let outs = kernel
                .send(vec![Item::new(
                    DEFAULT_ROOT_ID,
                    json!({"type": "create_agent", "id": CANVAS_ID, "parent_id": DEFAULT_ROOT_ID, "handler_module": "ram_store.tools"}),
                )])
                .await;
            assert!(
                matches!(outs[0].status, Status::Completed),
                "{}",
                outs[0].result
            );
        }
        place_dispatcher(&kernel).await;
        let view = kernel
            .agents
            .get(&fantastic_kernel::AgentId::from("dispatcher_view"))
            .expect("the dispatcher view is on the canvas");
        assert_eq!(view.handler_module.as_deref(), Some("html.tools"));
        assert_eq!(
            view.parent_id.as_ref().map(|p| p.0.as_str()),
            Some(CANVAS_ID)
        );
        drop(view);
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn place_dispatcher_without_canvas_leaves_no_view_and_no_panic() {
        let dir = tempfile::TempDir::new().unwrap();
        let tree = plan(&sample(), dir.path()).unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        apply(&kernel, &tree).await.unwrap();
        assert!(!exists(&kernel, CANVAS_ID));
        place_dispatcher(&kernel).await;
        assert!(!exists(&kernel, "dispatcher_view"));
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[test]
    fn chat_verbs_are_the_chat_ones_only() {
        assert_eq!(CHAT_VERBS, ["history", "send", "stop", "status", "watch"]);
        assert!(!CHAT_VERBS.contains(&"*"));
        assert_eq!(DISPATCHER_VIEW_ID, "dispatcher_view");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dispatcher_view_may_chat_with_workers() {
        let dir = tempfile::TempDir::new().unwrap();
        let tree = plan(&sample(), dir.path()).unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        apply(&kernel, &tree).await.unwrap();
        let canvas = json!({"type": "create_agent", "id": CANVAS_ID, "parent_id": DEFAULT_ROOT_ID, "handler_module": "ram_store.tools"});
        let outs = kernel.send(vec![Item::new(DEFAULT_ROOT_ID, canvas)]).await;
        assert!(
            matches!(outs[0].status, Status::Completed),
            "{}",
            outs[0].result
        );
        place_dispatcher(&kernel).await;
        // a worker and one of its subagents, made as the lead makes them
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
        let granted = grant_chat(&kernel, &["w1".to_string(), "nobody".to_string()]);
        assert_eq!(granted, 2);
        let grants = |id: &str, field: &str| {
            kernel
                .agents
                .get(&fantastic_kernel::AgentId::from(id))
                .map(|a| a.meta.read().unwrap()[field].clone())
                .unwrap_or(Value::Null)
        };
        for worker in ["w1", "w1_sub"] {
            assert_eq!(
                grants(worker, "callable_by")[DISPATCHER_VIEW_ID],
                json!(CHAT_VERBS),
                "{worker}"
            );
            assert_eq!(
                grants(DISPATCHER_VIEW_ID, "can_call")[worker],
                json!(CHAT_VERBS),
                "{worker}"
            );
        }
        // a second start is idempotent: the same grants, the same count (w1 and its subagent)
        assert_eq!(grant_chat(&kernel, &["w1".to_string()]), 2);
        assert_eq!(
            grants("w1", "callable_by")[DISPATCHER_VIEW_ID],
            json!(CHAT_VERBS)
        );
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }
}
