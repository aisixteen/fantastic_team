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
use crate::{board, dispatcher, worker_exec, worker_view};

/// The dispatcher's agent id: the team's main view, created at start.
pub const DISPATCHER_ID: &str = "dispatcher";
/// The canvas the dispatcher is placed on at the first start.
pub const CANVAS_ID: &str = "canvas_page";
/// The worker view agent, under the dispatcher; it opens a worker's view.
pub const WORKER_VIEW_ID: &str = "worker_view";
/// The board agent, under the root, when `team.yaml` has a `board` section.
pub const BOARD_ID: &str = "board";

/// Tools the binary always knows; a template may grant them without a definition.
pub const BUILTIN_TOOLS: [&str; 5] = [
    "worker_exec",
    "worker_files",
    "queue",
    "kernel",
    "kernel_state",
];

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
            "can_call": { (CANVAS_ID): "*", (WORKER_VIEW_ID): "*" },
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
    let view = format!("{DISPATCHER_ID}_view");
    if exists(kernel, &view) {
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
}
