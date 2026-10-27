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
use crate::{
    board, cadencer, dispatcher, pack_bundle, pack_listing, pack_panel, roles, worker_exec,
    worker_view,
};

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
/// The roles registry's default agent id (`roles.id` in `team.yaml` may name another).
pub const ROLES_ID: &str = "roles";
/// The task registry's agent id (fantastic's `task_registry`), a child of the lead,
/// present in every team with `roles:`.
pub const TASKS_ID: &str = config::TASKS_ID;
/// The verbs the `project_lead` card grants on the task registry (documentation: the
/// card is the source).
pub const PROJECT_LEAD_TASK_VERBS: [&str; 7] = [
    "add", "assign", "reassign", "status", "report", "list", "get",
];
/// What the first agent may call on the roles registry: ask the roles and the room
/// (`roles`, `role`, `get_count`), start an agent (`spawn`) and list who it started
/// (`fleet`). It does not set limits: those come from `team.yaml` at start.
pub const LEAD_ROLES_VERBS: [&str; 5] = ["roles", "role", "spawn", "get_count", "fleet"];
/// The kernel verbs a spawned agent may call on the root to wait for its children
/// (`await_set`, `await_cancel`): the project_lead card lists them.
pub const AWAIT_VERBS: [&str; 2] = ["await_set", "await_cancel"];
/// What the board may call on the roles registry (roles mode): who runs (`fleet`) and
/// the count (`get_count`).
pub const BOARD_ROLES_VERBS: [&str; 2] = ["fleet", "get_count"];
/// What the board may call on the task registry (roles mode): the rows.
pub const BOARD_TASKS_VERBS: [&str; 1] = ["list"];
/// What the board may call on the root (roles mode): `reflect`, for the project_lead's branch.
pub const BOARD_KERNEL_VERBS: [&str; 1] = ["reflect"];
/// What the dispatcher may call on the roles registry: the count it shows.
pub const DISPATCHER_ROLES_VERBS: [&str; 1] = ["get_count"];
/// The kernel verbs the dispatcher may call on the root to add and remove a worker
/// (`control` with `add_worker` / `remove_worker`) and to keep the groups lists current.
pub const DISPATCHER_KERNEL_VERBS: [&str; 3] = ["create_agent", "update_agent", "delete_agent"];
/// The page's agent: the dispatcher's html view on the canvas, made at the first start.
pub const DISPATCHER_VIEW_ID: &str = "dispatcher_view";
/// The pack browser panel's agent: an html view on the canvas, made at the first start.
pub const PACKS_VIEW_ID: &str = pack_panel::VIEW_ID;
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
    /// The team's one store agent (a `sqlite_store`), under the root.
    Memory,
    /// The credential store, under the root.
    Store,
    /// A tool, under the lead.
    Tool,
    /// The board: the team's cache of its repos, under the root.
    Board,
    /// The cadencer: the digests to the group drivers, under the root.
    Cadencer,
    /// The roles registry (`agentic_resources`, under the root), or the task registry (under the lead).
    Registry,
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

/// The role cards of a team with a `roles:` section: the folders load without an
/// error, every card names a tier that is a template, every card of the coder set
/// is there, and every grant key is an agent the team defines. `Ok` for a team
/// with no `roles:` section.
pub fn check_cards(cfg: &TeamConfig, team_dir: &Path) -> Result<(), String> {
    let Some(r) = &cfg.roles else {
        return Ok(());
    };
    let (cards, errors) = fantastic_agentic_resources::cards::load_dir(&role_dirs(r, team_dir));
    if !errors.is_empty() {
        return Err(format!("role cards: {}", errors.join("; ")));
    }
    let mut known: BTreeSet<&str> = BUILTIN_TOOLS.iter().copied().collect();
    known.insert(fantastic_agentic_resources::cards::REGISTRY_TOKEN);
    known.insert(cfg.queue.id.as_str());
    known.insert(TASKS_ID);
    known.extend(cfg.connectors.keys().map(String::as_str));
    if cfg.board.is_some() {
        known.insert(BOARD_ID);
    }
    for want in roles::CODER_ROLES {
        if !cards.iter().any(|c| c.name == want) {
            return Err(format!("role card `{want}` is missing"));
        }
    }
    for c in &cards {
        if !cfg.templates.contains_key(&c.tier) {
            return Err(format!(
                "role card `{}` names tier `{}` that is not a template in team.yaml",
                c.name, c.tier
            ));
        }
        if let Some(k) = c.grants.keys().find(|k| !known.contains(k.as_str())) {
            return Err(format!(
                "role card `{}` grants `{k}`, which team.yaml does not define",
                c.name
            ));
        }
    }
    Ok(())
}

/// The card folders of `roles.dirs`, made absolute under the team's directory.
fn role_dirs(r: &config::RolesCfg, team_dir: &Path) -> Vec<String> {
    r.dirs
        .iter()
        .map(|d| team_dir.join(d).to_string_lossy().into_owned())
        .collect()
}

/// The roles registry, under the root. The registry holds the tiers (one per
/// template, keyed by the template's name), the card folders and nothing else: the
/// limits are set at start (`apply_limits`). Every tier names the team's one store
/// agent (`cfg.store`) as its `memory_store_id`; the store keys each agent's history
/// by its own id. The store itself is planned by [`plan`], not here.
fn roles_plan(cfg: &TeamConfig, lead: &str, r: &config::RolesCfg, team_dir: &Path) -> Planned {
    let mem = cfg.store.id.as_str();
    let tiers: serde_json::Map<String, Value> = cfg
        .templates
        .iter()
        .map(|(n, t)| (n.clone(), config::tier_fields(t, mem)))
        .collect();
    Planned {
        id: r.id.clone(),
        parent: DEFAULT_ROOT_ID.into(),
        role: Role::Registry,
        record: json!({
            "type": "create_agent",
            "id": r.id,
            "parent_id": DEFAULT_ROOT_ID,
            "handler_module": fantastic_agentic_resources::HANDLER_MODULE,
            "role_dirs": role_dirs(r, team_dir),
            "tiers": tiers,
            "can_call": {(mem): "*"},
            "callable_by": {
                "$self": "*",
                "$parent": "*",
                (lead): LEAD_ROLES_VERBS,
                (DISPATCHER_ID): DISPATCHER_ROLES_VERBS,
            },
        }),
    }
}

/// The task registry (fantastic `task_registry`), a child of the lead so the lead
/// can admit a spawned project_lead on it (the card's admit step). It keeps its
/// rows in the team's one store agent, which keys them apart from the conversations.
fn tasks_plan(cfg: &TeamConfig, lead: &str) -> Planned {
    Planned {
        id: TASKS_ID.into(),
        parent: lead.into(),
        role: Role::Registry,
        record: json!({
            "type": "create_agent",
            "id": TASKS_ID,
            "parent_id": lead,
            "handler_module": fantastic_task_registry::HANDLER_MODULE,
            "task_store_id": cfg.store.id,
            "can_call": {(cfg.store.id.as_str()): "*"},
            "callable_by": {"$self": "*", "$parent": "*"},
        }),
    }
}

/// The planned agent `id` (the lead, planned first; it is there when a lead is).
fn planned_mut<'a>(out: &'a mut [Planned], id: &str) -> &'a mut Planned {
    out.iter_mut()
        .find(|p| p.id == id)
        .expect("the planned lead")
}

/// The team's ONE store agent (a `sqlite_store`, one file under the team directory).
/// The lead, the task registry (when the team has roles) and every group driver and
/// member may call it; each keeps its own rows, since the store keys rows by the
/// caller. An agent spawned later is added to its `callable_by` when it starts.
/// This is config: a `team.yaml` `store:` section may name another id or file.
fn store_plan(cfg: &TeamConfig) -> Planned {
    Planned {
        id: cfg.store.id.clone(),
        parent: DEFAULT_ROOT_ID.into(),
        role: Role::Memory,
        record: config::store_agent_record(&cfg.store, &store_callers(cfg)),
    }
}

/// The agents the plan lets call the store: the lead, `tasks` (with `roles:`) and every
/// group driver and member.
fn store_callers(cfg: &TeamConfig) -> Vec<String> {
    let mut callable: Vec<String> = cfg.lead_id().map(str::to_owned).into_iter().collect();
    if cfg.roles.is_some() {
        callable.push(TASKS_ID.to_string());
    }
    for g in cfg.groups.values() {
        callable.extend(g.driver.iter().cloned());
        callable.extend(g.members.iter().cloned());
    }
    callable
}

/// Make sure the store agent admits everyone the plan names, also when the store was
/// there before this start (the shell seeds its own `memory`, which `apply` reuses as it
/// is): merge the missing callers into its `callable_by` and keep every other key. An
/// operator write made at each start, so it is idempotent.
pub fn grant_store_callers(kernel: &Arc<Kernel>, cfg: &TeamConfig) -> Result<(), String> {
    for caller in store_callers(cfg) {
        grant_store(kernel, &cfg.store.id, &caller)?;
    }
    Ok(())
}

/// The plan, lead first. Validates the config and the grants.
pub fn plan(cfg: &TeamConfig, team_dir: &Path) -> Result<Vec<Planned>, String> {
    config::validate(cfg)?;
    check_grants(cfg)?;
    check_cards(cfg, team_dir)?;
    // The lead is the caller's: a file with no `first_agent` plans none, and the
    // tools hang under the root.
    let lead_id = cfg.lead_id().map(str::to_owned);
    let lead = lead_id.as_deref().unwrap_or(DEFAULT_ROOT_ID);
    let mut out = Vec::new();
    if let Some(f) = &cfg.first_agent {
        let tpl = &cfg.templates[&f.template];
        out.push(Planned {
            id: f.id.clone(),
            parent: DEFAULT_ROOT_ID.into(),
            role: Role::Lead,
            record: config::agent_record(tpl, &f.id, &cfg.store.id),
        });
    }
    out.push(store_plan(cfg));
    if let Some(s) = &cfg.credentials {
        out.push(Planned {
            id: s.id.clone(),
            parent: DEFAULT_ROOT_ID.into(),
            role: Role::Store,
            record: config::store_record(s),
        });
    }
    let mut dispatcher = Planned {
        id: DISPATCHER_ID.into(),
        parent: DEFAULT_ROOT_ID.into(),
        role: Role::Dispatcher,
        record: json!({
            "type": "create_agent",
            "id": DISPATCHER_ID,
            "parent_id": DEFAULT_ROOT_ID,
            "handler_module": dispatcher::HANDLER_MODULE,
            "board": BOARD_ID,
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
    };
    if let Some(l) = &lead_id {
        dispatcher.record["team"] = json!(l);
    }
    out.push(dispatcher);
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
        lead,
        worker_exec::worker_exec_record(team_dir),
    ));
    out.push(tool_plan(
        "worker_files",
        lead,
        json!({"handler_module": "file_bridge.tools", "root": work, "allow": ["."]}),
    ));
    // Only the pack agents may reach the files peer; the lead's own grants do not name it.
    // `allow_all_ingress` opens the edge (as the base `store` does); `callable_by` still
    // names who may call.
    let mut files = tool_plan(
        FILES_ID,
        lead,
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
    if let Some(l) = &lead_id {
        pack_can_call.insert(l.clone(), json!("*"));
    }
    // `install --parent-id` builds a pack's blueprint through `user.import_subtree`;
    // the other end of this edge is the `user` agent's `callable_by`, below.
    pack_can_call.insert(USER_ID.into(), json!(PACK_USER_VERBS));
    // Enabling a pack checks that its bound agents exist: the root's `list_agents`
    // (the root's own `callable_by` is written by `grant_agent_listing`).
    pack_can_call.insert(DEFAULT_ROOT_ID.into(), json!(PACK_ROOT_VERBS));
    for m in cfg.groups.values().flat_map(|g| g.members.iter()) {
        pack_can_call.insert(m.clone(), json!("*"));
    }
    // `eval {from_shifts}` reads the board's tasks for the measured record: the one
    // `board` verb, and only when the team has a board.
    if cfg.board.is_some() {
        pack_can_call.insert(BOARD_ID.into(), json!(PACK_BOARD_VERBS));
    }
    if let Some(l) = &lead_id {
        planned_mut(&mut out, l).record["callable_by"][PACK_ID] = json!("*");
    }
    out.push(pack_plan(
        PACK_ID,
        lead,
        json!({"handler_module": pack_bundle::HANDLER_MODULE, "can_call": pack_can_call}),
    ));
    // The `user` agent: its `import_subtree` builds a pack's blueprint. Only the pack
    // agent may call it, and only that verb.
    let mut user = tool_plan(
        USER_ID,
        lead,
        json!({"handler_module": fantastic_user::HANDLER_MODULE}),
    );
    user.record["callable_by"] = json!({"$self": "*", "$parent": "*", (PACK_ID): PACK_USER_VERBS});
    out.push(user);
    out.push(pack_plan(
        PACK_LISTING_ID,
        lead,
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
        lead,
        json!({"handler_module": "ram_store.tools"}),
    ));
    for (id, c) in &cfg.connectors {
        out.push(tool_plan(id, lead, config::connector_record(id, c)));
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
        if let Some(l) = &lead_id {
            planned_mut(&mut out, l).record["callable_by"][CADENCER_ID] = json!(["send"]);
        }
    }
    if let (Some(r), Some(l)) = (&cfg.roles, &lead_id) {
        // The lead starts agents through the registry (`spawn`), and the dispatcher
        // asks it for the count it shows. The registry's own `callable_by` names both.
        planned_mut(&mut out, l).record["can_call"][r.id.as_str()] = json!(LEAD_ROLES_VERBS);
        if let Some(d) = out.iter_mut().find(|p| p.id == DISPATCHER_ID) {
            d.record["can_call"][r.id.as_str()] = json!(DISPATCHER_ROLES_VERBS);
            d.record["roles"] = json!(r.id);
        }
        out.push(roles_plan(cfg, l, r, team_dir));
        out.push(tasks_plan(cfg, l));
        // The board lists the lead's tree, roles and tasks: it names the registries it
        // reads, and they name it. The root's end (`reflect`) is `grant_board_reflect`.
        if cfg.board.is_some() {
            for (id, verbs) in [
                (r.id.as_str(), json!(BOARD_ROLES_VERBS)),
                (TASKS_ID, json!(BOARD_TASKS_VERBS)),
            ] {
                if let Some(p) = out.iter_mut().find(|p| p.id == id) {
                    p.record["callable_by"][BOARD_ID] = verbs.clone();
                }
                if let Some(b) = out.iter_mut().find(|p| p.id == BOARD_ID) {
                    b.record["can_call"][id] = verbs;
                }
            }
            if let Some(b) = out.iter_mut().find(|p| p.id == BOARD_ID) {
                b.record["roles"] = json!(r.id);
                b.record["can_call"][DEFAULT_ROOT_ID] = json!(BOARD_KERNEL_VERBS);
            }
        }
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
            "callable_by": {"$self": "*", "$parent": "*", (DISPATCHER_ID): "*", (PACK_ID): PACK_BOARD_VERBS},
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
    if let Some(l) = cfg.lead_id() {
        can_call.insert(l.into(), json!(["send"]));
    }
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
    p.record["callable_by"] =
        json!({"$self": "*", "$parent": "*", "dispatcher": "*", (PACKS_VIEW_ID): "*"});
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

/// Set the registry's limits from `team.yaml` as the operator: `set_limit {max}` for the
/// total, `set_limit {role, max}` per role. A limit the registry holds that the file no
/// longer names is cleared (`max: null`). Idempotent: the same file gives the same
/// record. A refusal is returned, not swallowed. `Ok` for a team with no `roles:`.
pub async fn apply_limits(kernel: &Arc<Kernel>, cfg: &TeamConfig) -> Result<(), String> {
    let Some(r) = &cfg.roles else {
        return Ok(());
    };
    let held = kernel
        .agents
        .get(&fantastic_kernel::AgentId::from(r.id.as_str()))
        .map(|a| a.meta.read().expect("meta").get("limits").cloned())
        .ok_or_else(|| format!("limits: the roles registry `{}` is not there", r.id))?
        .unwrap_or(Value::Null);
    let mut calls: Vec<Value> = Vec::new();
    match r.limits.total {
        Some(n) => calls.push(json!({"type": "set_limit", "max": n})),
        None if held.get("total").is_some_and(|v| !v.is_null()) => {
            calls.push(json!({"type": "set_limit", "max": null}));
        }
        None => {}
    }
    for (role, n) in &r.limits.per_role {
        calls.push(json!({"type": "set_limit", "role": role, "max": n}));
    }
    if let Some(per) = held.get("per_role").and_then(Value::as_object) {
        for role in per.keys().filter(|k| !r.limits.per_role.contains_key(*k)) {
            calls.push(json!({"type": "set_limit", "role": role, "max": null}));
        }
    }
    for call in calls {
        let out = kernel
            .send(vec![Item::new(r.id.as_str(), call.clone())])
            .await
            .remove(0);
        if !matches!(out.status, Status::Completed) || out.result.get("error").is_some() {
            return Err(format!("limits: `{}` refused {call}: {}", r.id, out.result));
        }
    }
    kernel.flush_records().await;
    Ok(())
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

/// The one root (kernel) verb the pack agent may call: `list_agents`, to check
/// that the agents a pack is bound to are live before it is enabled.
pub const PACK_ROOT_VERBS: [&str; 1] = ["list_agents"];

/// Put the pack browser panel on the canvas, like [`place_dispatcher`]: an
/// `html.tools` view that may call the two pack agents. Best effort.
async fn place_packs_panel(kernel: &Arc<Kernel>) {
    let view = PACKS_VIEW_ID;
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
                "html": pack_panel::HTML,
                "can_call": { (PACK_ID): "*", (PACK_LISTING_ID): "*" },
            }),
        )])
        .await;
    if let Some(o) = outs.pop() {
        if o.result.get("error").is_some() || !matches!(o.status, Status::Completed) {
            eprintln!("packs panel not placed on `{CANVAS_ID}`: {}", o.result);
        }
    }
}

/// Let the pack agent call `list_agents` on the root: the root's end of the edge
/// the pack's `can_call` names. An operator write made at each start, so it is
/// idempotent; a refusal is reported, not fatal. True when the root now grants it.
pub fn grant_agent_listing(kernel: &Arc<Kernel>) -> bool {
    let root = fantastic_kernel::AgentId::from(DEFAULT_ROOT_ID);
    let verbs = json!(PACK_ROOT_VERBS);
    match kernel.edit_callable_by(&root, |cb| {
        if cb.get(PACK_ID) == Some(&verbs) {
            return false;
        }
        cb.insert(PACK_ID.into(), verbs.clone());
        true
    }) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("pack agent: root list_agents grant: {e}");
            false
        }
    }
}

/// The `user` agent (`user.tools`): it holds the blueprint verbs.
pub const USER_ID: &str = "user";
/// The one `user` verb the pack agent may call (`install --parent-id`).
pub const PACK_USER_VERBS: [&str; 1] = ["import_subtree"];
/// The one `board` verb the pack agent may call (`eval {from_shifts}`).
pub const PACK_BOARD_VERBS: [&str; 1] = ["board"];
/// The one `pack` verb the dispatcher page may call (its Pack install panel).
pub const PAGE_PACK_VERBS: [&str; 1] = ["install"];

/// Grant the page's agent the `install` verb on the pack agent, on both ends (the
/// view's `can_call` and the pack's `callable_by`). Idempotent. Returns whether
/// both ends hold the grant; false when the view or the pack agent is not live.
pub fn grant_pack_install(kernel: &Arc<Kernel>) -> bool {
    if !exists(kernel, DISPATCHER_VIEW_ID) || !exists(kernel, PACK_ID) {
        return false;
    }
    let verbs = json!(PAGE_PACK_VERBS);
    let by_pack = kernel.edit_callable_by(&fantastic_kernel::AgentId::from(PACK_ID), |cb| {
        if cb.get(DISPATCHER_VIEW_ID) == Some(&verbs) {
            return false;
        }
        cb.insert(DISPATCHER_VIEW_ID.into(), verbs.clone());
        true
    });
    let by_view =
        kernel.edit_can_call(&fantastic_kernel::AgentId::from(DISPATCHER_VIEW_ID), |cc| {
            if cc.get(PACK_ID) == Some(&verbs) {
                return false;
            }
            cc.insert(PACK_ID.into(), verbs.clone());
            true
        });
    match (by_pack, by_view) {
        (Ok(_), Ok(_)) => true,
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("dispatcher pack install: {e}");
            false
        }
    }
}

/// The two `user` verbs the page uses for the chat's questions: the open
/// `inquiries` of an asker and `answer_inquiry`. Granted by [`grant_inquiries`].
pub const INQUIRY_VERBS: [&str; 2] = ["inquiries", "answer_inquiry"];

/// Grant the page's agent the inquiry verbs on the `user` agent, on both ends like
/// [`grant_chat`]: the view's `can_call` names `user` and `user`'s `callable_by`
/// names the view. Idempotent; never `*`. Skipped when `user` or the view is not
/// live. Returns true when a grant was written.
pub fn grant_inquiries(kernel: &Arc<Kernel>) -> bool {
    if !exists(kernel, DISPATCHER_VIEW_ID) || !exists(kernel, USER_ID) {
        return false;
    }
    let verbs = json!(INQUIRY_VERBS);
    let view = fantastic_kernel::AgentId::from(DISPATCHER_VIEW_ID);
    let user = fantastic_kernel::AgentId::from(USER_ID);
    let by_user = kernel.edit_callable_by(&user, |cb| {
        if cb.get(DISPATCHER_VIEW_ID) == Some(&verbs) {
            return false;
        }
        cb.insert(DISPATCHER_VIEW_ID.into(), verbs.clone());
        true
    });
    let by_view = kernel.edit_can_call(&view, |cc| {
        if cc.get(USER_ID) == Some(&verbs) {
            return false;
        }
        cc.insert(USER_ID.into(), verbs.clone());
        true
    });
    match (by_user, by_view) {
        (Ok(a), Ok(b)) => a || b,
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("dispatcher inquiries: `{USER_ID}`: {e}");
            false
        }
    }
}

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

/// Name every agent below the root in the root's `callable_by` with the await verbs
/// ([`AWAIT_VERBS`]), the other end of a spawned agent's `kernel` grant (its card):
/// the id of a spawned agent is not known before it exists. An operator write made at
/// each start, so it is idempotent. Only for a team with a roles registry.
pub fn grant_await(kernel: &Arc<Kernel>, cfg: &TeamConfig) -> Result<(), String> {
    let Some(r) = &cfg.roles else {
        return Ok(());
    };
    if !exists(kernel, &r.id) {
        return Ok(());
    }
    let verbs = json!(AWAIT_VERBS);
    let root = fantastic_kernel::AgentId::from(DEFAULT_ROOT_ID);
    kernel
        .edit_callable_by(&root, |cb| {
            if cb.get("$subtree") == Some(&verbs) {
                return false;
            }
            cb.insert("$subtree".into(), verbs.clone());
            true
        })
        .map(|_| ())
        .map_err(|e| format!("await verbs: {e}"))
}

/// Name the board in the root's `callable_by` with `reflect` ([`BOARD_KERNEL_VERBS`]), the
/// other end of the board's `can_call` on the root: it reads the project_lead's branch.
/// An operator write made at each start, so it is idempotent. Only for a team with a
/// roles registry and a board.
pub fn grant_board_reflect(kernel: &Arc<Kernel>, cfg: &TeamConfig) -> Result<(), String> {
    if cfg.roles.is_none() || !exists(kernel, BOARD_ID) {
        return Ok(());
    }
    let verbs = json!(BOARD_KERNEL_VERBS);
    let root = fantastic_kernel::AgentId::from(DEFAULT_ROOT_ID);
    kernel
        .edit_callable_by(&root, |cb| {
            if cb.get(BOARD_ID) == Some(&verbs) {
                return false;
            }
            cb.insert(BOARD_ID.into(), verbs.clone());
            true
        })
        .map(|_| ())
        .map_err(|e| format!("board reflect: {e}"))
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
    if instructions.is_some() && cfg.first_agent.is_none() {
        return Err(
            "instructions need a first_agent in team.yaml: the caller defines the agents".into(),
        );
    }
    let tree = plan(&cfg, dir)?;
    let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    rt.block_on(async {
        let kernel = crate::start(dir).await.map_err(|e| e.to_string())?;
        let res: Result<Applied, String> = async {
            let applied = apply(&kernel, &tree).await?;
            if applied.created.iter().any(|c| c == DISPATCHER_ID) {
                place_dispatcher(&kernel).await;
                place_packs_panel(&kernel).await;
            }
            grant_agent_listing(&kernel);
            let workers: Vec<String> = cfg
                .groups
                .values()
                .flat_map(|g| g.members.iter().cloned())
                .collect();
            grant_chat(&kernel, &workers);
            grant_control(&kernel, &workers);
            grant_pack_install(&kernel);
            if let Err(e) = grant_agent_verbs(&kernel) {
                eprintln!("{e}");
            }
            grant_inquiries(&kernel);
            apply_limits(&kernel, &cfg).await?;
            if let Err(e) = grant_await(&kernel, &cfg) {
                eprintln!("{e}");
            }
            if let Err(e) = grant_store_callers(&kernel, &cfg) {
                eprintln!("{e}");
            }
            if let Err(e) = grant_board_reflect(&kernel, &cfg) {
                eprintln!("{e}");
            }
            if let (Some(text), Some(f)) = (instructions, &cfg.first_agent) {
                if instructions_due(&applied, &f.id, resend) {
                    deliver_instructions(&kernel, &f.id, text).await?;
                }
            }
            Ok(applied)
        }
        .await;
        kernel.shutdown().await;
        fantastic_sqlite_store::close_all(&kernel);
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
                "memory",
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

    fn repo_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn example_team_yaml_validates() {
        let cfg =
            config::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/team.yaml"))
                .unwrap();
        // the example's `roles` folder is the repo's own
        let tree = plan(&cfg, &repo_root()).unwrap();
        assert_eq!(tree[0].id, "lead");
        assert!(tree.iter().any(|p| p.id == "roles"));
    }

    #[test]
    fn plan_contains_no_secret_values() {
        let cfg =
            config::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/team.yaml"))
                .unwrap();
        let tree = plan(&cfg, &repo_root()).unwrap();
        let text =
            serde_json::to_string(&tree.iter().map(|p| &p.record).collect::<Vec<_>>()).unwrap();
        assert!(!text.contains("sk-"));
        assert!(!text.contains("Bearer "));
    }

    const EMPTY_IDS: [&str; 10] = [
        "memory",
        "dispatcher",
        "worker_view",
        "worker_exec",
        "worker_files",
        "files",
        "pack",
        "user",
        "pack_listing",
        "queue",
    ];

    fn empty_cfg() -> TeamConfig {
        parse("{}").unwrap()
    }

    const WORKER_ONLY: &str = "templates:\n  worker:\n    handler_module: ram_store.tools\n    model: m\n    endpoint: http://e\n    can_call: [queue]\nfirst_agent:\n  id: w1\n  template: worker\n";

    #[test]
    fn empty_config_plans_only_system_agents() {
        let tree = plan(&empty_cfg(), Path::new("/team")).unwrap();
        let got = ids(&tree);
        for id in EMPTY_IDS {
            assert!(
                got.iter().any(|i| i == id),
                "the plan has no `{id}`: {got:?}"
            );
        }
        assert!(tree.iter().all(|p| p.role != Role::Lead));
        // The team's one store is a system agent, planned with or without a lead.
        assert_eq!(planned(&tree, "memory").role, Role::Memory);
        assert!(!got
            .iter()
            .any(|i| i == "lead" || i == ROLES_ID || i == TASKS_ID));
        for p in &tree {
            assert!(
                p.parent == DEFAULT_ROOT_ID || p.parent == DISPATCHER_ID,
                "{} hangs under {}",
                p.id,
                p.parent
            );
        }
    }

    #[test]
    fn empty_config_tools_hang_under_the_root() {
        let tree = plan(&empty_cfg(), Path::new("/team")).unwrap();
        for id in ["queue", "pack", "files", "user", "pack_listing"] {
            let p = planned(&tree, id);
            assert_eq!(p.parent, DEFAULT_ROOT_ID, "{id}");
            assert_eq!(p.record["parent_id"], DEFAULT_ROOT_ID, "{id}");
        }
        assert!(planned(&tree, "dispatcher").record.get("team").is_none());
        let pack = planned(&tree, "pack");
        assert!(pack.record["can_call"].get("lead").is_none());
    }

    #[test]
    fn worker_only_plan_has_no_lead_role_and_no_registry() {
        let cfg = parse(WORKER_ONLY).unwrap();
        let tree = plan(&cfg, Path::new("/team")).unwrap();
        assert_eq!(tree[0].id, "w1");
        let got = ids(&tree);
        assert!(!got.iter().any(|i| i == ROLES_ID || i == TASKS_ID));
        assert_eq!(planned(&tree, "dispatcher").record["team"], "w1");
        assert_eq!(planned(&tree, "queue").parent, "w1");
        assert!(tree[0].record.get("can_call").is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn apply_of_empty_config_creates_no_coding_agent() {
        let dir = tempfile::TempDir::new().unwrap();
        let tree = plan(&empty_cfg(), dir.path()).unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        let applied = apply(&kernel, &tree).await.unwrap();
        assert_eq!(applied.created, ids(&tree));
        for p in &tree {
            assert!(p.record.get("model").is_none(), "{}", p.id);
            let h = p.record["handler_module"].as_str().unwrap_or("");
            assert!(
                !h.contains("anthropic") && !h.contains("vllm"),
                "{} is an AI agent: {h}",
                p.id
            );
        }
        for id in &applied.created {
            assert!(EMPTY_IDS.contains(&id.as_str()), "unexpected `{id}`");
        }
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[test]
    fn instructions_without_a_first_agent_are_refused() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join(config::FILE_NAME), "{}\n").unwrap();
        let e = run_at_start(dir.path(), Some("hi"), false).unwrap_err();
        assert_eq!(
            e,
            "instructions need a first_agent in team.yaml: the caller defines the agents"
        );
        assert!(run_at_start(dir.path(), None, false).unwrap().is_some());
    }

    #[test]
    fn run_at_start_empty_config_starts_clean() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join(config::FILE_NAME), "{}\n").unwrap();
        let applied = run_at_start(dir.path(), None, false).unwrap().unwrap();
        assert!(!applied.created.is_empty());
        let tree = plan(&empty_cfg(), dir.path()).unwrap();
        for id in &applied.created {
            let p = planned(&tree, id);
            assert!(p.record.get("model").is_none(), "{id} has a model");
        }
        assert!(!applied.created.iter().any(|c| c == "lead"));
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
            json!({"files": "*", "lead": "*", "user": ["import_subtree"], "kernel_state": ["list_agents"]})
        );
        assert_eq!(
            pack.record["callable_by"],
            json!({"$self": "*", "$parent": "*", "dispatcher": "*", "packs_view": "*"}),
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
            json!({"$self": "*", "$parent": "*", "dispatcher": "*", "packs_view": "*"}),
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

    /// The applied team with one packed worker `w` (one eval, id `a`) and a
    /// callable member `w1`: what the MCP `send` and `reflect` legs reach.
    async fn applied_pack_with_one_eval() -> (tempfile::TempDir, Arc<Kernel>) {
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
    async fn mcp_docs_of_the_pack_agent_lists_eval_with_its_args() {
        let (dir, kernel) = applied_pack_with_one_eval().await;
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
        let (dir, kernel) = applied_pack_with_one_eval().await;
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
    fn plan_with_roles_and_board_lets_the_board_read_the_registries() {
        let dir = roles_team_dir();
        let tree = plan(&roles_cfg(ROLES_LIMITS), dir.path()).unwrap();
        let board = planned(&tree, BOARD_ID);
        assert_eq!(board.record["roles"], "roles");
        assert_eq!(
            board.record["can_call"]["roles"],
            json!(["fleet", "get_count"])
        );
        assert_eq!(board.record["can_call"][TASKS_ID], json!(["list"]));
        assert_eq!(
            board.record["can_call"][DEFAULT_ROOT_ID],
            json!(["reflect"])
        );
        assert_eq!(
            planned(&tree, "roles").record["callable_by"][BOARD_ID],
            json!(["fleet", "get_count"])
        );
        assert_eq!(
            planned(&tree, TASKS_ID).record["callable_by"][BOARD_ID],
            json!(["list"])
        );
    }

    #[test]
    fn plan_without_roles_leaves_the_board_record_unchanged() {
        let tree = plan(
            &parse(&with_groups_and_board()).unwrap(),
            Path::new("/team"),
        )
        .unwrap();
        let board = planned(&tree, BOARD_ID);
        assert!(board.record.get("roles").is_none());
        assert_eq!(board.record["can_call"], json!({"git": "*", "queue": "*"}));
    }

    #[test]
    fn pack_can_call_the_board_for_shift_outcomes() {
        let tree = plan(
            &parse(&with_groups_and_board()).unwrap(),
            Path::new("/team"),
        )
        .unwrap();
        let pack = tree.iter().find(|p| p.id == PACK_ID).unwrap();
        assert_eq!(pack.record["can_call"]["board"], json!(["board"]));
        let board = tree.iter().find(|p| p.id == "board").unwrap();
        assert_eq!(board.record["callable_by"][PACK_ID], json!(["board"]));
        let no_board = plan(&sample(), Path::new("/team")).unwrap();
        let pack = no_board.iter().find(|p| p.id == PACK_ID).unwrap();
        assert!(pack.record["can_call"].get("board").is_none());
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
    fn pack_agent_can_list_agents_on_the_kernel() {
        let cfg = parse(&sample_yaml()).unwrap();
        let tree = plan(&cfg, Path::new("/team")).unwrap();
        let pack = tree.iter().find(|p| p.id == PACK_ID).unwrap();
        assert_eq!(
            pack.record["can_call"][DEFAULT_ROOT_ID],
            json!(["list_agents"])
        );
        // that verb only; the listing agent has no such edge
        let listing = tree.iter().find(|p| p.id == PACK_LISTING_ID).unwrap();
        assert!(listing.record["can_call"].get(DEFAULT_ROOT_ID).is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn grant_agent_listing_opens_the_roots_end_once() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        let by = |kernel: &Arc<Kernel>| {
            kernel
                .agents
                .get(&fantastic_kernel::AgentId::from(DEFAULT_ROOT_ID))
                .map(|a| a.meta.read().unwrap()["callable_by"].clone())
                .unwrap_or(Value::Null)
        };
        assert!(by(&kernel).get(PACK_ID).is_none());
        assert!(grant_agent_listing(&kernel));
        assert_eq!(by(&kernel)[PACK_ID], json!(["list_agents"]));
        assert!(grant_agent_listing(&kernel), "idempotent");
        assert_eq!(by(&kernel)[PACK_ID], json!(["list_agents"]));
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn place_packs_panel_puts_view_on_the_canvas() {
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
        place_packs_panel(&kernel).await;
        let view = kernel
            .agents
            .get(&fantastic_kernel::AgentId::from("packs_view"))
            .expect("the packs view is on the canvas");
        assert_eq!(view.handler_module.as_deref(), Some("html.tools"));
        assert_eq!(
            view.parent_id.as_ref().map(|p| p.0.as_str()),
            Some(CANVAS_ID)
        );
        let meta = view.meta.read().unwrap().clone();
        assert_eq!(meta["can_call"][PACK_ID], "*");
        assert_eq!(meta["can_call"][PACK_LISTING_ID], "*");
        assert_eq!(meta["html"], pack_panel::HTML);
        drop(view);
        // a second call leaves the one view
        place_packs_panel(&kernel).await;
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn place_packs_panel_without_canvas_is_not_fatal() {
        let dir = tempfile::TempDir::new().unwrap();
        let tree = plan(&sample(), dir.path()).unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        apply(&kernel, &tree).await.unwrap();
        assert!(!exists(&kernel, CANVAS_ID));
        place_packs_panel(&kernel).await;
        assert!(!exists(&kernel, "packs_view"));
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[test]
    fn plan_pack_agents_name_the_packs_view() {
        let cfg = parse(&sample_yaml()).unwrap();
        let tree = plan(&cfg, Path::new("/team")).unwrap();
        assert_eq!(PACKS_VIEW_ID, "packs_view");
        for id in [PACK_ID, PACK_LISTING_ID] {
            let a = tree.iter().find(|p| p.id == id).unwrap();
            assert_eq!(a.record["callable_by"][PACKS_VIEW_ID], "*", "{id}");
        }
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

    #[tokio::test(flavor = "multi_thread")]
    async fn the_page_may_call_install_on_the_pack_agent_only() {
        let dir = tempfile::tempdir().unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        assert!(!grant_pack_install(&kernel), "nothing live yet");
        let tree = plan(&sample(), dir.path()).unwrap();
        apply(&kernel, &tree).await.unwrap();
        for rec in [
            json!({"type": "create_agent", "id": CANVAS_ID, "parent_id": DEFAULT_ROOT_ID, "handler_module": "ram_store.tools"}),
        ] {
            kernel.send(vec![Item::new(DEFAULT_ROOT_ID, rec)]).await;
        }
        place_dispatcher(&kernel).await;
        assert!(grant_pack_install(&kernel));
        assert!(grant_pack_install(&kernel), "idempotent");
        let field = |id: &str, f: &str| {
            kernel
                .agents
                .get(&fantastic_kernel::AgentId::from(id))
                .map(|a| a.meta.read().unwrap()[f].clone())
                .unwrap_or(Value::Null)
        };
        assert_eq!(
            field(PACK_ID, "callable_by")[DISPATCHER_VIEW_ID],
            json!(["install"])
        );
        assert_eq!(
            field(DISPATCHER_VIEW_ID, "can_call")[PACK_ID],
            json!(["install"])
        );
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn grant_inquiries_writes_both_ends_without_star() {
        let dir = tempfile::TempDir::new().unwrap();
        let tree = plan(&sample(), dir.path()).unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        apply(&kernel, &tree).await.unwrap();
        let canvas = json!({"type": "create_agent", "id": CANVAS_ID, "parent_id": DEFAULT_ROOT_ID, "handler_module": "ram_store.tools"});
        kernel.send(vec![Item::new(DEFAULT_ROOT_ID, canvas)]).await;
        // no page agent yet: nothing is written
        assert!(!exists(&kernel, DISPATCHER_VIEW_ID));
        assert!(!grant_inquiries(&kernel));
        place_dispatcher(&kernel).await;
        assert!(
            exists(&kernel, USER_ID),
            "the team kernel has a `user` agent"
        );
        assert!(grant_inquiries(&kernel));
        let grants = |id: &str, field: &str| {
            kernel
                .agents
                .get(&fantastic_kernel::AgentId::from(id))
                .map(|a| a.meta.read().unwrap()[field].clone())
                .unwrap_or(Value::Null)
        };
        let want = json!(["inquiries", "answer_inquiry"]);
        assert_eq!(grants(DISPATCHER_VIEW_ID, "can_call")[USER_ID], want);
        assert_eq!(grants(USER_ID, "callable_by")[DISPATCHER_VIEW_ID], want);
        assert!(!serde_json::to_string(&want).unwrap().contains('*'));
        assert!(!grants(DISPATCHER_VIEW_ID, "can_call")
            .as_object()
            .unwrap()
            .contains_key("*"));
        assert!(!grant_inquiries(&kernel), "idempotent");
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    // ---- the roles registry (#185) ----

    /// A roles-enabled team: the three tiers the cards name, the `github` connector and
    /// the board the project_lead card grants, the registry with the limits `limits`.
    /// `lead_handler` is the first agent's handler module.
    fn roles_yaml(lead_handler: &str, roles: &str) -> String {
        let quoted = |p: std::path::PathBuf| p.to_string_lossy().replace('\'', "''");
        let tmp = std::env::temp_dir();
        let t = |h: &str, can_call: &str| {
            format!("    handler_module: {h}\n    model: m\n    endpoint: http://e\n    can_call: {can_call}\n")
        };
        format!(
            "templates:\n  strong:\n{}  small:\n{}  issue_keeper:\n{}first_agent:\n  template: strong\nconnectors:\n  git:\n    kind: team_git.tools\n    record:\n      remote: origin\n      repo_dir: '{}'\n      workspace_root: '{}'\n      default_branch: main\n  github:\n    kind: team_github.tools\n    record:\n      app:\n        credentials:\n          store: credentials\n          ref: github/app\n      repos: [o/r]\n      git: git\n      directive: Push a work branch.\nqueue:\n  id: queue\nboard:\n  repos: [o/r]\n  github: github\n{roles}",
            t(lead_handler, "[worker_exec, worker_files, queue]"),
            t("ram_store.tools", "[worker_exec, worker_files, queue]"),
            t("ram_store.tools", "[queue]"),
            quoted(tmp.join("clone")),
            quoted(tmp.join("wt")),
        )
    }

    const ROLES_LIMITS: &str = "roles:\n  limits:\n    total: 3\n    per_role:\n      worker: 2\n";

    /// A team directory holding a copy of the repo's `roles/` folder.
    fn roles_team_dir() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../roles");
        let to = dir.path().join("roles");
        std::fs::create_dir_all(&to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
        dir
    }

    fn roles_cfg(roles: &str) -> TeamConfig {
        parse(&roles_yaml("ram_store.tools", roles)).unwrap()
    }

    fn planned<'a>(tree: &'a [Planned], id: &str) -> &'a Planned {
        tree.iter()
            .find(|p| p.id == id)
            .unwrap_or_else(|| panic!("the plan has no `{id}`"))
    }

    #[test]
    fn plan_without_roles_is_unchanged() {
        let tree = plan(&sample(), Path::new("/team")).unwrap();
        assert!(!ids(&tree)
            .iter()
            .any(|i| i == ROLES_ID || i == "roles_memory"));
        assert_eq!(ids(&tree).iter().filter(|i| *i == "memory").count(), 1);
        assert!(planned(&tree, "dispatcher").record.get("roles").is_none());
        assert!(planned(&tree, "dispatcher").record["can_call"]
            .get(ROLES_ID)
            .is_none());
        assert!(tree[0].record["can_call"].get(ROLES_ID).is_none());
        assert!(!ids(&tree).iter().any(|i| i == TASKS_ID));
        // no cards are read when the team has no `roles:` section
        assert_eq!(check_cards(&sample(), Path::new("/no/such/dir")), Ok(()));
    }

    #[test]
    fn plan_with_roles_adds_registry_and_grants() {
        let dir = roles_team_dir();
        let tree = plan(&roles_cfg(ROLES_LIMITS), dir.path()).unwrap();
        let ids = ids(&tree);
        assert_eq!(&ids[ids.len() - 2..], ["roles", "tasks"]);
        assert!(!ids.iter().any(|i| i == "roles_memory"), "{ids:?}");
        let mem = planned(&tree, "memory");
        assert_eq!(mem.parent, DEFAULT_ROOT_ID);
        assert_eq!(mem.record["handler_module"], "sqlite_store.tools");
        assert_eq!(
            mem.record["callable_by"],
            json!({"$self": "*", "lead": "*", "tasks": "*"})
        );
        let reg = planned(&tree, "roles");
        assert_eq!(reg.parent, DEFAULT_ROOT_ID);
        assert_eq!(reg.role, Role::Registry);
        assert_eq!(
            reg.record["handler_module"],
            fantastic_agentic_resources::HANDLER_MODULE
        );
        let want_dir = dir.path().join("roles").to_string_lossy().into_owned();
        assert_eq!(reg.record["role_dirs"], json!([want_dir]));
        let tiers = reg.record["tiers"].as_object().unwrap();
        let keys: Vec<&str> = tiers.keys().map(String::as_str).collect();
        assert_eq!(keys, ["issue_keeper", "small", "strong"]);
        assert_eq!(
            tiers["small"],
            config::tier_fields(&roles_cfg("roles: {}\n").templates["small"], "memory")
        );
        assert_eq!(tiers["small"]["memory_store_id"], "memory");
        assert!(
            reg.record.get("limits").is_none(),
            "limits are set at start"
        );
        assert_eq!(reg.record["can_call"], json!({"memory": "*"}));
        assert_eq!(
            reg.record["callable_by"],
            json!({
                "$self": "*", "$parent": "*",
                "lead": ["roles", "role", "spawn", "get_count", "fleet"],
                "dispatcher": ["get_count"],
                "board": ["fleet", "get_count"],
            })
        );
        assert_eq!(
            tree[0].record["can_call"]["roles"],
            json!(["roles", "role", "spawn", "get_count", "fleet"])
        );
        assert!(tree[0].record["can_call"].get("kernel").is_none());
        let d = planned(&tree, "dispatcher");
        assert_eq!(d.record["roles"], "roles");
        assert_eq!(d.record["can_call"]["roles"], json!(["get_count"]));
        // another id comes from the config
        let other = plan(&roles_cfg("roles:\n  id: reg\n"), dir.path()).unwrap();
        assert_eq!(planned(&other, "dispatcher").record["roles"], "reg");
        assert!(!other.iter().any(|p| p.id.ends_with("_memory")));
    }

    #[test]
    fn plan_refuses_a_card_with_an_unknown_tier() {
        let dir = roles_team_dir();
        let w = dir.path().join("roles/worker.md");
        let text = std::fs::read_to_string(&w).unwrap();
        std::fs::write(&w, text.replace("tier: small", "tier: tiny")).unwrap();
        let e = plan(&roles_cfg(ROLES_LIMITS), dir.path()).unwrap_err();
        assert_eq!(
            e,
            "role card `worker` names tier `tiny` that is not a template in team.yaml"
        );
    }

    #[test]
    fn plan_refuses_a_card_with_an_unknown_grant() {
        let dir = roles_team_dir();
        let w = dir.path().join("roles/worker.md");
        let text = std::fs::read_to_string(&w).unwrap();
        std::fs::write(&w, text.replace("  queue: \"*\"\n", "  mystery: \"*\"\n")).unwrap();
        let e = plan(&roles_cfg(ROLES_LIMITS), dir.path()).unwrap_err();
        assert_eq!(
            e,
            "role card `worker` grants `mystery`, which team.yaml does not define"
        );
        // a team with no github connector cannot hold the project_lead card's grants
        let bare = parse(
            &roles_yaml("ram_store.tools", "roles: {}\n")
                .replace("board:\n  repos: [o/r]\n  github: github\n", ""),
        )
        .unwrap();
        let ok = roles_team_dir();
        let mut no_gh = bare.clone();
        no_gh.connectors.remove("github");
        let e = plan(&no_gh, ok.path()).unwrap_err();
        assert_eq!(
            e,
            "role card `project_lead` grants `github`, which team.yaml does not define"
        );
    }

    #[test]
    fn plan_refuses_a_missing_coder_card_and_a_bad_card() {
        let dir = roles_team_dir();
        std::fs::remove_file(dir.path().join("roles/spec_reviewer.md")).unwrap();
        let e = plan(&roles_cfg(ROLES_LIMITS), dir.path()).unwrap_err();
        assert_eq!(e, "role card `spec_reviewer` is missing");
        std::fs::write(dir.path().join("roles/broken.md"), "no header\n").unwrap();
        let e = plan(&roles_cfg(ROLES_LIMITS), dir.path()).unwrap_err();
        assert_eq!(e, "role cards: card broken.md: missing header");
        let empty = tempfile::TempDir::new().unwrap();
        let e = plan(&roles_cfg(ROLES_LIMITS), empty.path()).unwrap_err();
        assert!(e.starts_with("role cards: role dir "), "{e}");
    }

    #[test]
    fn plan_with_roles_adds_the_task_registry() {
        let dir = roles_team_dir();
        let tree = plan(&roles_cfg(ROLES_LIMITS), dir.path()).unwrap();
        let t = planned(&tree, "tasks");
        assert_eq!(t.parent, "lead");
        assert_eq!(t.role, Role::Registry);
        assert_eq!(t.record["parent_id"], "lead");
        assert_eq!(
            t.record["handler_module"],
            fantastic_task_registry::HANDLER_MODULE
        );
        assert_eq!(t.record["task_store_id"], "memory");
        assert_eq!(t.record["can_call"], json!({"memory": "*"}));
        assert_eq!(
            t.record["callable_by"],
            json!({"$self": "*", "$parent": "*", "board": ["list"]})
        );
        let at = |i: &str| ids(&tree).iter().position(|x| x == i).unwrap();
        assert!(at("lead") < at("tasks"), "the lead exists first");
        // another registry id still names the team's one store
        let other = plan(&roles_cfg("roles:\n  id: reg\n"), dir.path()).unwrap();
        assert_eq!(planned(&other, "tasks").record["task_store_id"], "memory");
    }

    #[test]
    fn a_card_may_grant_the_task_registry() {
        let dir = roles_team_dir();
        assert_eq!(check_cards(&roles_cfg(ROLES_LIMITS), dir.path()), Ok(()));
        let card = std::fs::read_to_string(dir.path().join("roles/project_lead.md")).unwrap();
        assert!(card.contains("  tasks: [add"));
        // an unknown grant key is still refused
        let bad = card.replace("  tasks: [add", "  nope: [add");
        std::fs::write(dir.path().join("roles/project_lead.md"), bad).unwrap();
        let e = check_cards(&roles_cfg(ROLES_LIMITS), dir.path()).unwrap_err();
        assert_eq!(
            e,
            "role card `project_lead` grants `nope`, which team.yaml does not define"
        );
    }

    // ---- one store agent for the whole team (#185) ----

    /// The planned agents whose handler is the durable store.
    fn stores(tree: &[Planned]) -> Vec<&Planned> {
        tree.iter()
            .filter(|p| p.record["handler_module"] == "sqlite_store.tools")
            .collect()
    }

    const GROUPS: &str =
        "groups:\n  g1:\n    driver: lead\n    members: [w1, w2]\n  g2:\n    members: [w3]\n";

    #[test]
    fn plan_has_exactly_one_store_agent() {
        let dir = roles_team_dir();
        let tree = plan(&roles_cfg(ROLES_LIMITS), dir.path()).unwrap();
        let s = stores(&tree);
        assert_eq!(s.len(), 1, "{:?}", ids(&tree));
        assert_eq!(s[0].id, "memory");
        assert_eq!(s[0].parent, DEFAULT_ROOT_ID);
        assert_eq!(s[0].role, Role::Memory);
        // no per-agent memory any more, and no RAM store but the queue (the lead's handler here is a stand-in)
        assert!(!ids(&tree).iter().any(|i| i.ends_with("_memory")));
        let ram: Vec<&str> = tree
            .iter()
            .filter(|p| p.record["handler_module"] == "ram_store.tools" && p.role != Role::Lead)
            .map(|p| p.id.as_str())
            .collect();
        assert_eq!(ram, ["queue"]);
        // the store is created right after the lead
        assert_eq!(ids(&tree)[..2], ["lead", "memory"]);
    }

    #[test]
    fn every_ai_agent_and_tasks_use_the_one_store() {
        let dir = roles_team_dir();
        let tree = plan(&roles_cfg(ROLES_LIMITS), dir.path()).unwrap();
        assert_eq!(planned(&tree, "lead").record["memory_store_id"], "memory");
        assert_eq!(planned(&tree, "lead").record["can_call"]["memory"], "*");
        let reg = planned(&tree, "roles");
        let tiers = reg.record["tiers"].as_object().unwrap();
        assert_eq!(tiers.len(), 3);
        for (n, t) in tiers {
            assert_eq!(t["memory_store_id"], "memory", "tier {n}");
        }
        assert_eq!(reg.record["can_call"], json!({"memory": "*"}));
        let tasks = planned(&tree, "tasks");
        assert_eq!(tasks.record["task_store_id"], "memory");
        assert_eq!(tasks.record["can_call"], json!({"memory": "*"}));
        // every planned record that names a memory or task store names the one store
        for p in &tree {
            for key in ["memory_store_id", "task_store_id"] {
                if let Some(v) = p.record.get(key) {
                    assert_eq!(v, "memory", "{}.{key}", p.id);
                }
            }
        }
    }

    #[test]
    fn store_callable_by_names_lead_tasks_and_group_members() {
        let dir = roles_team_dir();
        let text = format!("{}{GROUPS}", roles_yaml("ram_store.tools", ROLES_LIMITS));
        let tree = plan(&parse(&text).unwrap(), dir.path()).unwrap();
        assert_eq!(
            planned(&tree, "memory").record["callable_by"],
            json!({"$self": "*", "lead": "*", "tasks": "*", "w1": "*", "w2": "*", "w3": "*"})
        );
        // without roles: no `tasks`
        let text = format!("{}{GROUPS}", sample_yaml());
        let tree = plan(&parse(&text).unwrap(), dir.path()).unwrap();
        assert_eq!(
            planned(&tree, "memory").record["callable_by"],
            json!({"$self": "*", "lead": "*", "w1": "*", "w2": "*", "w3": "*"})
        );
    }

    #[test]
    fn plan_without_roles_has_one_store_and_no_registry() {
        let tree = plan(&sample(), Path::new("/team")).unwrap();
        assert_eq!(stores(&tree).len(), 1);
        assert_eq!(planned(&tree, "lead").record["memory_store_id"], "memory");
        assert!(!ids(&tree).iter().any(|i| i == ROLES_ID || i == TASKS_ID));
        assert_eq!(
            planned(&tree, "memory").record["callable_by"],
            json!({"$self": "*", "lead": "*"})
        );
    }

    #[test]
    fn a_team_yaml_may_name_a_different_store() {
        let dir = roles_team_dir();
        let text = format!(
            "{}store:\n  id: s2\n  path: .fantastic/other.db\n",
            roles_yaml("ram_store.tools", ROLES_LIMITS)
        );
        let tree = plan(&parse(&text).unwrap(), dir.path()).unwrap();
        assert_eq!(stores(&tree).len(), 1);
        let s = planned(&tree, "s2");
        assert_eq!(s.record["path"], ".fantastic/other.db");
        assert!(!ids(&tree).iter().any(|i| i == "memory"));
        assert_eq!(planned(&tree, "lead").record["memory_store_id"], "s2");
        let tiers = planned(&tree, "roles").record["tiers"].clone();
        for (n, t) in tiers.as_object().unwrap() {
            assert_eq!(t["memory_store_id"], "s2", "tier {n}");
        }
        assert_eq!(planned(&tree, "tasks").record["task_store_id"], "s2");
        assert_eq!(
            planned(&tree, "tasks").record["can_call"],
            json!({"s2": "*"})
        );
        assert_eq!(
            planned(&tree, "roles").record["can_call"],
            json!({"s2": "*"})
        );
    }

    #[test]
    fn store_path_is_one_file_per_team_dir() {
        let tree = plan(&sample(), Path::new("/team")).unwrap();
        // relative: the kernel resolves it under the team directory it runs in
        assert_eq!(
            planned(&tree, "memory").record["path"],
            ".fantastic/memory.db"
        );
        assert_eq!(stores(&tree).len(), 1);
        let dir = roles_team_dir();
        let tree = plan(&roles_cfg(ROLES_LIMITS), dir.path()).unwrap();
        let paths: Vec<&Value> = tree.iter().filter_map(|p| p.record.get("path")).collect();
        assert_eq!(paths, [&json!(".fantastic/memory.db")]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn history_and_registry_rows_survive_a_restart() {
        let dir = roles_team_dir();
        let kernel = caller_team(dir.path(), ROLES_LIMITS).await;
        let rows = json!([
            {"kind": "user", "content": "plan the release"},
            {"kind": "assistant", "content": "spawning a worker"},
        ]);
        let appended = call_as(
            &kernel,
            "lead",
            "memory",
            json!({"type": "append", "rows": rows}),
        )
        .await;
        assert_eq!(appended["appended"], 2, "{appended}");
        let add = json!({"type": "add", "id": "o/r#9", "source": "gh:o/r#9", "holder": "lead"});
        let task = call_as(&kernel, "lead", "tasks", add).await;
        assert!(task.get("error").is_none(), "{task}");
        let read = json!({"type": "read"});
        let hist_before = call_as(&kernel, "lead", "memory", read.clone()).await;
        let list = json!({"type": "list"});
        let tasks_before = call_as(&kernel, "lead", "tasks", list.clone()).await;
        assert!(tasks_before.to_string().contains("o/r#9"), "{tasks_before}");
        kernel.flush_records().await;
        kernel.shutdown().await;
        fantastic_sqlite_store::close_all(&kernel);
        bootstrap::shutdown(dir.path()).unwrap();
        assert!(dir.path().join(".fantastic/memory.db").is_file());
        drop(kernel);

        let kernel = caller_team(dir.path(), ROLES_LIMITS).await;
        let hist_after = call_as(&kernel, "lead", "memory", read).await;
        assert_eq!(hist_after["rows"], hist_before["rows"], "{hist_after}");
        let contents: Vec<&str> = hist_after["rows"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|r| r["content"].as_str())
            .collect();
        assert!(contents.contains(&"plan the release"), "{hist_after}");
        assert!(contents.contains(&"spawning a worker"), "{hist_after}");
        let tasks_after = call_as(&kernel, "lead", "tasks", list).await;
        assert_eq!(tasks_after, tasks_before);
        kernel.shutdown().await;
        fantastic_sqlite_store::close_all(&kernel);
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_store_that_was_there_before_still_admits_the_plans_callers() {
        let dir = roles_team_dir();
        let kernel = crate::start(dir.path()).await.unwrap();
        // the shell seeds its own `memory`; the plan reuses it as it is
        let seeded = ask(
            &kernel,
            DEFAULT_ROOT_ID,
            json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID, "id": "memory",
                   "handler_module": "sqlite_store.tools", "path": ".fantastic/memory.db",
                   "callable_by": {"user": ["*"]}}),
        )
        .await;
        assert!(seeded.get("error").is_none(), "{seeded}");
        let cfg = roles_cfg(ROLES_LIMITS);
        let applied = apply(&kernel, &plan(&cfg, dir.path()).unwrap())
            .await
            .unwrap();
        assert!(applied.reused.iter().any(|i| i == "memory"));
        let before = meta_value(&kernel, "memory")["callable_by"].clone();
        assert!(before.get("tasks").is_none(), "{before}");
        grant_store_callers(&kernel, &cfg).unwrap();
        let by = meta_value(&kernel, "memory")["callable_by"].clone();
        assert_eq!(by["lead"], "*", "{by}");
        assert_eq!(by["tasks"], "*", "{by}");
        assert_eq!(by["user"], json!(["*"]), "the seeded key stays: {by}");
        // idempotent
        grant_store_callers(&kernel, &cfg).unwrap();
        assert_eq!(meta_value(&kernel, "memory")["callable_by"], by);
        kernel.shutdown().await;
        fantastic_sqlite_store::close_all(&kernel);
        bootstrap::shutdown(dir.path()).unwrap();
    }

    /// Spawn a project_lead and a worker as the lead, and admit the project_lead on `tasks`.
    async fn lead_with_project_lead(kernel: &Arc<Kernel>) -> (String, String) {
        let spawn = |role: &'static str| {
            call_as(
                kernel,
                "lead",
                "roles",
                json!({"type": "spawn", "role": role}),
            )
        };
        let pl = spawn("project_lead").await["id"]
            .as_str()
            .unwrap()
            .to_string();
        let w = spawn("worker").await["id"].as_str().unwrap().to_string();
        (pl, w)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_spawned_project_lead_records_a_handed_out_issue() {
        let dir = roles_team_dir();
        let kernel = caller_team(dir.path(), ROLES_LIMITS).await;
        let (pl, w) = lead_with_project_lead(&kernel).await;
        let pl_meta = meta_value(&kernel, &pl);
        assert_eq!(
            pl_meta["can_call"]["tasks"],
            json!(PROJECT_LEAD_TASK_VERBS),
            "the card's grant reached the spawned agent"
        );
        // not admitted yet: refused by the gate
        let add = json!({"type": "add", "id": "o/r#7", "source": "gh:o/r#7", "holder": w});
        let denied = call_as(&kernel, &pl, "tasks", add.clone()).await;
        assert_eq!(denied["reason"], "unauthorized", "{denied}");
        // the admit step is an update_agent of `tasks`.callable_by; the lead's template
        // grant for it is team config, so the operator does it here
        let mut by = meta_value(&kernel, "tasks")["callable_by"].clone();
        by[pl.as_str()] = json!("*");
        let up = ask(
            &kernel,
            "kernel_state",
            json!({"type": "update_agent", "id": "tasks", "callable_by": by}),
        )
        .await;
        assert!(up.get("error").is_none(), "{up}");
        let e = call_as(&kernel, &pl, "tasks", add).await;
        assert!(e.get("error").is_none(), "{e}");
        assert_eq!(e["source"], "gh:o/r#7", "{e}");
        assert_eq!(e["update_to"], "gh:o/r#7");
        assert_eq!(e["holder"], json!(w));
        assert_eq!(e["status"], "waiting");
        assert_eq!(e["history"].as_array().unwrap().len(), 1);
        let re = call_as(
            &kernel,
            &pl,
            "tasks",
            json!({"type": "reassign", "id": "o/r#7", "holder": {"subtree": pl}}),
        )
        .await;
        assert!(re.get("error").is_none(), "{re}");
        let hist = re["history"].as_array().unwrap();
        assert_eq!(hist.len(), 2, "{re}");
        assert_eq!(hist[1]["from"], json!(w), "{re}");
        let got = call_as(&kernel, &pl, "tasks", json!({"type": "get", "id": "o/r#7"})).await;
        assert_eq!(got["holder"], json!({"subtree": pl}), "{got}");
        let list = call_as(
            &kernel,
            &pl,
            "tasks",
            json!({"type": "list", "holder": {"subtree": pl}}),
        )
        .await;
        assert!(list.to_string().contains("o/r#7"), "{list}");
        // the rows live in the team's one store
        let dump = ask(&kernel, "memory", json!({"type": "dump"})).await;
        assert!(dump.to_string().contains("o/r#7"), "{dump}");
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_worker_cannot_write_the_task_registry() {
        let dir = roles_team_dir();
        let kernel = caller_team(dir.path(), ROLES_LIMITS).await;
        let (pl, w) = lead_with_project_lead(&kernel).await;
        // the worker card grants nothing on `tasks`, and `tasks` admits no worker
        let wm = meta_value(&kernel, &w);
        assert!(wm["can_call"].get("tasks").is_none(), "{wm}");
        assert!(meta_value(&kernel, "tasks")["callable_by"]
            .get(w.as_str())
            .is_none());
        // a project_lead the lead has not admitted is refused by the gate
        let add = json!({"type": "add", "id": "o/r#8", "source": "gh:o/r#8", "holder": w});
        let denied = call_as(&kernel, &pl, "tasks", add).await;
        assert_eq!(denied["reason"], "unauthorized", "{denied}");
        let got = call_as(&kernel, "lead", "tasks", json!({"type": "list"})).await;
        assert!(!got.to_string().contains("o/r#8"), "{got}");
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    async fn call_as(kernel: &Arc<Kernel>, caller: &str, target: &str, payload: Value) -> Value {
        kernel
            .send(vec![Item::new(
                caller,
                json!({"type": "call", "target": target, "payload": payload}),
            )])
            .await
            .remove(0)
            .result
    }

    async fn ask(kernel: &Arc<Kernel>, target: &str, payload: Value) -> Value {
        kernel
            .send(vec![Item::new(target, payload)])
            .await
            .remove(0)
            .result
    }

    fn meta_value(kernel: &Arc<Kernel>, id: &str) -> Value {
        let a = kernel
            .agents
            .get(&fantastic_kernel::AgentId::from(id))
            .unwrap_or_else(|| panic!("no agent `{id}`"));
        let m = a.meta.read().unwrap().clone();
        Value::Object(m)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn config_enables_the_registry_and_loads_the_five_cards() {
        let dir = roles_team_dir();
        let cfg = roles_cfg(ROLES_LIMITS);
        let tree = plan(&cfg, dir.path()).unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        apply(&kernel, &tree).await.unwrap();
        let out = ask(&kernel, "roles", json!({"type": "roles"})).await;
        assert_eq!(out["errors"], json!([]), "{out}");
        let mut names: Vec<String> = out["roles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["name"].as_str().unwrap().to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "issue_keeper",
                "issue_writer",
                "lead",
                "project_lead",
                "spec_reviewer",
                "worker"
            ]
        );
        let r = ask(&kernel, "roles", json!({"type": "reflect"})).await;
        assert!(r.to_string().contains("enabled"), "{r}");
        let pl = ask(
            &kernel,
            "roles",
            json!({"type": "role", "name": "project_lead"}),
        )
        .await;
        assert_eq!(pl["tier"], "strong");
        assert!(
            pl["grants"]["github"].is_array(),
            "the coder card replaced the generic: {pl}"
        );
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    /// An agent that makes the call it is told to AS ITSELF, so the gate and `spawn`
    /// see a real lead.
    struct CallerBundle;

    #[async_trait::async_trait]
    impl fantastic_kernel::bundle::Bundle for CallerBundle {
        fn name(&self) -> &str {
            "caller"
        }

        async fn handle(
            &self,
            _agent_id: &fantastic_kernel::AgentId,
            payload: &Value,
            kernel: &Arc<Kernel>,
        ) -> Result<fantastic_kernel::bundle::Reply, fantastic_kernel::bundle::BundleError>
        {
            match payload["type"].as_str() {
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

    /// The roles team on a kernel whose first agent is a caller.
    async fn caller_team(dir: &Path, roles: &str) -> Arc<Kernel> {
        let mut reg = crate::registry();
        reg.register("caller.tools", CallerBundle);
        let mut booted =
            bootstrap::bootstrap(reg, bootstrap::BootstrapOptions::one_shot(dir)).unwrap();
        fantastic_kernel::lock::acquire(dir).unwrap();
        booted.write_seed().unwrap();
        let kernel = booted.kernel.clone();
        crate::boot_kernel(&kernel).await.unwrap();
        let cfg = parse(&roles_yaml("caller.tools", roles)).unwrap();
        apply(&kernel, &plan(&cfg, dir).unwrap()).await.unwrap();
        kernel
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lead_spawn_goes_through_the_registry() {
        let dir = roles_team_dir();
        let kernel = caller_team(dir.path(), ROLES_LIMITS).await;
        let out = call_as(
            &kernel,
            "lead",
            "roles",
            json!({"type": "spawn", "role": "worker"}),
        )
        .await;
        let id = out["id"]
            .as_str()
            .unwrap_or_else(|| panic!("{out}"))
            .to_string();
        assert_eq!(out["role"], "worker");
        assert_eq!(out["tier"], "small");
        let m = meta_value(&kernel, &id);
        assert_eq!(m["spawned_by"], "roles");
        assert_eq!(m["role"], "worker");
        assert_eq!(m["tier"], "small");
        assert_eq!(m["memory_store_id"], "memory");
        let card =
            fantastic_agentic_resources::cards::parse_card(roles::WORKER, "worker.md").unwrap();
        assert_eq!(m["system_prompt"], json!(card.directive));
        let child = kernel
            .agents
            .get(&fantastic_kernel::AgentId::from(id.as_str()))
            .unwrap()
            .parent_id
            .clone();
        assert_eq!(child.unwrap().as_str(), "lead", "a child of the lead");
        // the card's grants: the exec and file tools and the queue, plus the shared store
        for t in ["worker_exec", "worker_files", "queue", "memory"] {
            assert_eq!(m["can_call"][t], "*", "{t}");
        }
        // and the store admits it, so it keeps its own rows there
        let store = meta_value(&kernel, "memory");
        assert_eq!(store["callable_by"][id.as_str()], "*", "{store}");
        // the lead holds pause/resume/stop on it, and the child admits only that
        let verbs = json!(["pause", "resume", "stop"]);
        let lead = meta_value(&kernel, "lead");
        assert_eq!(lead["can_call"][id.as_str()], verbs);
        assert_eq!(m["callable_by"]["lead"], verbs);
        // the lead has no way to create an agent itself
        assert!(lead["can_call"].get("kernel").is_none());
        let direct = call_as(
            &kernel,
            "lead",
            "kernel",
            json!({"type": "create_agent", "id": "sneak", "handler_module": "ram_store.tools"}),
        )
        .await;
        assert!(direct["error"].is_string(), "{direct}");
        assert!(!kernel
            .agents
            .contains_key(&fantastic_kernel::AgentId::from("sneak")));
        // a spawn does not open the tools to the child: the lead admits it
        let q = meta_value(&kernel, "queue");
        assert!(q["callable_by"].get(id.as_str()).is_none(), "{q}");
        // another caller cannot spawn
        let denied = call_as(
            &kernel,
            id.as_str(),
            "roles",
            json!({"type": "spawn", "role": "worker"}),
        )
        .await;
        assert!(denied["error"].is_string(), "{denied}");
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn boot_limits_are_applied() {
        let dir = roles_team_dir();
        let kernel = caller_team(dir.path(), ROLES_LIMITS).await;
        apply_limits(&kernel, &roles_cfg(ROLES_LIMITS))
            .await
            .unwrap();
        let total = ask(&kernel, "roles", json!({"type": "get_count"})).await;
        assert_eq!(total, json!({"running": 0, "max": 3}));
        let w = ask(
            &kernel,
            "roles",
            json!({"type": "get_count", "role": "worker"}),
        )
        .await;
        assert_eq!(w, json!({"running": 0, "max": 2}));
        let spawn = |role: &'static str| {
            call_as(
                &kernel,
                "lead",
                "roles",
                json!({"type": "spawn", "role": role}),
            )
        };
        assert!(spawn("worker").await["id"].is_string());
        assert!(spawn("worker").await["id"].is_string());
        let third = spawn("worker").await;
        assert_eq!(third["error"], "limit", "{third}");
        assert_eq!(third["scope"], "role");
        assert!(
            third["reason"].as_str().unwrap().contains("at its limit"),
            "{third}"
        );
        assert!(spawn("issue_keeper").await["id"].is_string());
        let fourth = spawn("issue_keeper").await;
        assert_eq!(fourth["error"], "limit", "{fourth}");
        assert_eq!(fourth["scope"], "total");
        assert_eq!(
            fourth["reason"],
            "all roles together are at the limit (3 running, max 3)"
        );
        let total = ask(&kernel, "roles", json!({"type": "get_count"})).await;
        assert_eq!(total, json!({"running": 3, "max": 3}));
        // a limit the file no longer names is cleared
        apply_limits(&kernel, &roles_cfg("roles: {}\n"))
            .await
            .unwrap();
        let total = ask(&kernel, "roles", json!({"type": "get_count"})).await;
        assert_eq!(total, json!({"running": 3, "max": null}));
        let w = ask(
            &kernel,
            "roles",
            json!({"type": "get_count", "role": "worker"}),
        )
        .await;
        assert_eq!(w, json!({"running": 2, "max": null}));
        assert!(meta_value(&kernel, "roles")
            .get("limits")
            .is_none_or(|l| l.as_object().is_some_and(|m| m.is_empty())));
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_board_may_reflect_the_root_in_roles_mode() {
        let dir = roles_team_dir();
        let kernel = caller_team(dir.path(), "roles: {}\n").await;
        let cfg = roles_cfg("roles: {}\n");
        assert!(meta_value(&kernel, DEFAULT_ROOT_ID)["callable_by"]
            .get(BOARD_ID)
            .is_none());
        grant_board_reflect(&kernel, &cfg).unwrap();
        grant_board_reflect(&kernel, &cfg).unwrap();
        assert_eq!(
            meta_value(&kernel, DEFAULT_ROOT_ID)["callable_by"][BOARD_ID],
            json!(["reflect"])
        );
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_spawned_project_lead_may_await_its_children() {
        let dir = roles_team_dir();
        let kernel = caller_team(dir.path(), "roles: {}\n").await;
        grant_await(&kernel, &roles_cfg("roles: {}\n")).unwrap();
        let root = meta_value(&kernel, DEFAULT_ROOT_ID);
        assert_eq!(
            root["callable_by"]["$subtree"],
            json!(["await_set", "await_cancel"])
        );
        grant_await(&kernel, &roles_cfg("roles: {}\n")).unwrap();
        assert_eq!(
            meta_value(&kernel, DEFAULT_ROOT_ID)["callable_by"]["$subtree"],
            root["callable_by"]["$subtree"]
        );
        // the project_lead the lead spawns waits for its own child with await_set
        let pl = call_as(
            &kernel,
            "lead",
            "roles",
            json!({"type": "spawn", "role": "project_lead"}),
        )
        .await;
        let pl = pl["id"]
            .as_str()
            .unwrap_or_else(|| panic!("{pl}"))
            .to_string();
        let w = call_as(
            &kernel,
            "lead",
            "roles",
            json!({"type": "spawn", "role": "worker"}),
        )
        .await;
        let w = w["id"].as_str().unwrap().to_string();
        let out = kernel
            .send(vec![Item::new(
                pl.as_str(),
                json!({"type": "call", "target": "kernel_state",
                    "payload": {"type": "await_set", "ids": [w], "timeout_s": 5}}),
            )])
            .await
            .remove(0)
            .result;
        assert!(out.get("error").is_none(), "{out}");
        // no grant for a team with no registry
        let mut none = roles_cfg("roles: {}\n");
        none.roles = None;
        let dir2 = tempfile::TempDir::new().unwrap();
        let k2 = crate::start(dir2.path()).await.unwrap();
        grant_await(&k2, &none).unwrap();
        assert!(meta_value(&k2, DEFAULT_ROOT_ID)["callable_by"]
            .get("$subtree")
            .is_none());
        k2.shutdown().await;
        bootstrap::shutdown(dir2.path()).unwrap();
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn apply_limits_is_idempotent_and_refusals_are_returned() {
        let dir = roles_team_dir();
        let cfg = roles_cfg(ROLES_LIMITS);
        let tree = plan(&cfg, dir.path()).unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        apply(&kernel, &tree).await.unwrap();
        apply_limits(&kernel, &cfg).await.unwrap();
        let first = meta_value(&kernel, "roles")["limits"].clone();
        assert_eq!(first, json!({"total": 3, "per_role": {"worker": 2}}));
        apply_limits(&kernel, &cfg).await.unwrap();
        assert_eq!(meta_value(&kernel, "roles")["limits"], first);
        // a team with no `roles:` has nothing to apply
        apply_limits(&kernel, &sample()).await.unwrap();
        // a registry that refuses (no role_dirs: disabled) is an error, not silence
        ask(
            &kernel,
            "kernel",
            json!({"type": "update_agent", "id": "roles", "role_dirs": null}),
        )
        .await;
        let e = apply_limits(&kernel, &cfg).await.unwrap_err();
        assert!(e.starts_with("limits: `roles` refused"), "{e}");
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }
}
