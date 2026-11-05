//! fantastic_team as a library: this repo's bundles ([`register_team`]) for
//! fantastic's terminal-shell library (`fantastic_tui::main_with`) and an
//! in-process kernel ([`start`]) for tests. The command line is fantastic's,
//! unchanged: `main.rs` passes it to `fantastic_tui::main_with`. Config is
//! fantastic's too: agent records (`create_agent`) and recipe arguments.
#![deny(missing_docs)]

pub mod agent_views;
pub mod app_control;
pub mod board;
pub mod cadence_feed;
pub mod cadencer;
pub mod declared_views;
pub mod dispatcher;
pub mod driver;
pub mod driver_config;
pub mod driver_rules;
pub mod new_team;
pub mod pack_browser;
pub mod pack_cli;
pub mod pack_eval;
pub mod pack_hosted;
pub mod pack_listing;
pub mod pack_manifest;
pub mod pack_page;
pub mod pack_record;
pub mod pack_scan;
pub mod page;
pub mod peer_envelope;
pub mod peer_transport;
pub mod peers;
pub mod puller;
pub mod repo_config;
pub mod setup_github;
pub mod tree;
pub mod watchdog;
pub mod webhook;
pub mod webui;
pub mod worker_view;

use std::path::Path;
use std::sync::Arc;

use fantastic_kernel::bootstrap::{self, BootstrapOptions, DEFAULT_ROOT_ID};
use fantastic_kernel::{lock, BundleRegistry, Item, Kernel, Status};
use serde_json::json;

pub mod worker_exec;

/// This repo's bundles, added to the registry of fantastic's shell AFTER its
/// stock set (an `fantastic_tui::Extra`): the git and github connectors. The
/// worker exec restriction ([`worker_exec`]), the config ([`config`]) and the
/// role cards ([`roles`]) are records and files, not bundles; the roles registry
/// (`agentic_resources.tools`, crates/agentic_resources) is inert in a team whose
/// registry record has no `role_dirs`; the task registry (`task_registry.tools`,
/// crates/task_registry) is inert in a team whose registry record has no
/// `task_store_id`.
pub fn register_team(reg: &mut BundleRegistry) {
    reg.register(
        team_git_connector::HANDLER_MODULE,
        team_git_connector::GitConnectorBundle,
    );
    reg.register(
        team_github_connector::HANDLER_MODULE,
        team_github_connector::GithubConnectorBundle,
    );
    reg.register(
        team_agentic_resources::HANDLER_MODULE,
        team_agentic_resources::AgenticResourcesBundle,
    );
    reg.register(
        team_task_registry::HANDLER_MODULE,
        team_task_registry::TaskRegistryBundle::default(),
    );
    // The model backend and the credential store the new team's records name (`new_team`).
    reg.register(
        fantastic_anthropic_backend::HANDLER_MODULE,
        fantastic_anthropic_backend::AnthropicBundle,
    );
    reg.register(
        fantastic_credential_store::HANDLER_MODULE,
        fantastic_credential_store::CredentialStoreBundle::default(),
    );
    reg.register(puller::HANDLER_MODULE, puller::PullerBundle::default());
    // The team page, for the web UI that `--port` persists (`app_control::persist_web`).
    reg.register(page::HANDLER_MODULE, page::PageBundle);
    reg.register(pack_listing::HANDLER_MODULE, pack_listing::ListingBundle);
    reg.register(webhook::HANDLER_MODULE, webhook::WebhookBundle::default());
    reg.register(board::HANDLER_MODULE, board::BoardBundle);
    reg.register(pack_bundle::HANDLER_MODULE, pack_bundle::PackBundle);
    reg.register(dispatcher::HANDLER_MODULE, dispatcher::DispatcherBundle);
    reg.register(
        cadencer::HANDLER_MODULE,
        cadencer::CadencerBundle::default(),
    );
    reg.register(
        worker_view::HANDLER_MODULE,
        worker_view::WorkerViewBundle::default(),
    );
}

/// A kernel registry for [`start`] (tests, embedding): the bundles the base
/// tree needs (the root's state, the file store, the RAM memory store, the
/// durable `sqlite_store`, the exec bridge) plus [`register_team`]. fantastic's shell builds its own, fuller
/// set when the binary runs; that one is private to it.
pub fn registry() -> BundleRegistry {
    let mut reg = BundleRegistry::new();
    reg.register(
        fantastic_core::HANDLER_MODULE,
        fantastic_core::KernelStateBundle::default(),
    );
    reg.register("file_bridge.tools", fantastic_file::FileBundle::default());
    reg.register(
        fantastic_ram_store::HANDLER_MODULE,
        fantastic_ram_store::RamStoreBundle::default(),
    );
    // The durable memory store: the team's one store agent (`config::StoreCfg`).
    // The caller closes it with `fantastic_sqlite_store::close_all` after the shutdown.
    reg.register(
        fantastic_sqlite_store::HANDLER_MODULE,
        fantastic_sqlite_store::SqliteStoreBundle,
    );
    reg.register(
        fantastic_exec_bridge::HANDLER_MODULE,
        fantastic_exec_bridge::ExecBridgeBundle,
    );
    // `user.tools`: the blueprint verbs (`import_subtree`) behind `packs install --parent-id`.
    reg.register(
        fantastic_user::HANDLER_MODULE,
        fantastic_user::UserBundle::default(),
    );
    // The web host, its socket leg and the team page, for a handle that serves a port.
    reg.register(fantastic_web::HANDLER_MODULE, fantastic_web::WebBundle);
    reg.register(
        fantastic_web_ws::HANDLER_MODULE,
        fantastic_web_ws::WebWsBundle,
    );
    register_team(&mut reg);
    reg
}

/// The README for AIs (`docs/ai-readme.md`), built into the binary. It is printed
/// by `fantastic_team ai-readme`.
pub const AI_README: &str = include_str!("../../../docs/ai-readme.md");

/// The commands that run once and hold no kernel of their own: `send`, `readme`,
/// `import` and the rest of fantastic's one-shot words, plus `mcp` (its stdout is
/// its protocol). The binary starts no shutdown watchdog for them.
pub const ONE_SHOT: &[&str] = &[
    "mcp",
    "send",
    "readme",
    "import",
    "migrate-home",
    "__ctrl_break",
];

/// Whether `argv` is one of the [`ONE_SHOT`] commands.
pub fn is_one_shot(argv: &[String]) -> bool {
    argv.first().is_some_and(|a| ONE_SHOT.contains(&a.as_str()))
}

/// Seed a fresh directory's base tree: a RAM memory store for the kernel's
/// execution rows, and a file-bridge store so records persist. The file
/// bridge is open only to the kernel and its persistence loader
/// (`kernel_state`). The root is pointed at the memory store. A root that
/// already names a `memory_store_id` is left alone. A directory seeded by
/// `fantastic_tui::seed_dir` already holds the two stores (its blueprint has
/// `kernel_memory`): then only the root is pointed at the memory store.
/// Returns `Ok(true)` when it seeded, `Ok(false)` when there was nothing to
/// do. Mirrors the blueprint in fantastic's terminal app.
pub async fn seed_base_tree(kernel: &Arc<Kernel>) -> Result<bool, String> {
    let root = kernel
        .send(vec![Item::new(DEFAULT_ROOT_ID, json!({"type": "reflect"}))])
        .await;
    let first = match root.first() {
        Some(o) if matches!(o.status, Status::Completed) => o,
        Some(o) => return Err(format!("base tree: reflect failed: {}", o.result)),
        None => return Err("base tree: reflect failed: no reply".to_string()),
    };
    let reflected = first.result.clone();
    if reflected
        .get("memory_store_id")
        .and_then(|v| v.as_str())
        .is_some()
    {
        return Ok(false);
    }
    let steps = [
        (
            DEFAULT_ROOT_ID,
            json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID,
                   "id": "kernel_memory", "handler_module": "ram_store.tools",
                   "admins": ["kernel"]}),
        ),
        (
            DEFAULT_ROOT_ID,
            json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID,
                   "id": "store", "handler_module": "file_bridge.tools",
                   "root": ".fantastic", "allow": ["."],
                   "ingress_rule": "allow_all_ingress",
                   "callable_by": {"kernel_state": "*", "kernel": "*"}}),
        ),
        (
            "kernel",
            json!({"type": "update_agent", "id": DEFAULT_ROOT_ID,
                   "memory_store_id": "kernel_memory"}),
        ),
    ];
    // The shell's blueprint wrote the stores: keep them, point the root at the memory.
    let shell_seeded = kernel
        .agents
        .contains_key(&fantastic_kernel::AgentId::from("kernel_memory"));
    let skip = if shell_seeded { 2 } else { 0 };
    for (target, payload) in steps.into_iter().skip(skip) {
        for o in kernel.send(vec![Item::new(target, payload)]).await {
            if !matches!(o.status, Status::Completed) {
                return Err(format!("base tree seeding failed: {}", o.result));
            }
        }
    }
    kernel.flush_records().await;
    Ok(true)
}

/// Boot a headless kernel in `dir` and start it: the daemon sequence of
/// fantastic-cli mode 5, with a base tree seeded into a fresh directory. The
/// caller stops it with `kernel.shutdown()` and then `bootstrap::shutdown(dir)`.
pub async fn start(dir: &Path) -> Result<Arc<Kernel>, Box<dyn std::error::Error>> {
    let mut booted = bootstrap::bootstrap(registry(), BootstrapOptions::one_shot(dir))?;
    lock::acquire(dir)?;
    let kernel = booted.kernel.clone();
    let outcome = match booted.write_seed() {
        Ok(()) => boot_kernel(&kernel).await,
        Err(e) => Err(e.to_string()),
    };
    match outcome {
        Ok(()) => Ok(kernel),
        Err(msg) => {
            kernel.shutdown().await;
            fantastic_sqlite_store::close_all(&kernel);
            bootstrap::shutdown(dir)?;
            Err(msg.into())
        }
    }
}

/// Everything after the lock: seed the base tree, then start the kernel.
/// Any error here makes `start` shut down and release the lock.
async fn boot_kernel(kernel: &Arc<Kernel>) -> Result<(), String> {
    let seeded = seed_base_tree(kernel).await?;
    let started = kernel.start(true).await;
    if started
        .get("error")
        .filter(|e| fantastic_bundle::py_truthy(e))
        .is_some()
    {
        return Err(fantastic_bundle::py_str(&started["error"]));
    }
    if seeded {
        persist_seeded_records(kernel).await?;
    }
    Ok(())
}

/// The seed ran before the kernel's record writer subscribed, so the seeded
/// agents were never queued for it. Touch each one now with an `update_agent`
/// that re-sets a value the seed already wrote, so its record lands on disk.
/// The touches never change a security field, so the record keeps the seed's
/// own values. Without the touch a restart finds the root pointing at a
/// `kernel_memory` that has no record.
async fn persist_seeded_records(kernel: &Arc<Kernel>) -> Result<(), String> {
    for (id, key, value) in [
        ("kernel_memory", "admins", json!(["kernel"])),
        ("store", "root", json!(".fantastic")),
    ] {
        let mut payload = json!({"type": "update_agent", "id": id});
        payload[key] = value;
        for o in kernel.send(vec![Item::new("kernel", payload)]).await {
            if !matches!(o.status, Status::Completed) {
                return Err(format!("base tree: persisting {id} failed: {}", o.result));
            }
        }
    }
    kernel.flush_records().await;
    Ok(())
}

pub mod pack_bundle;
pub mod pack_registry;
pub mod roles;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn register_team_registers_the_anthropic_backend_and_the_credential_store() {
        let mut reg = BundleRegistry::new();
        register_team(&mut reg);
        assert!(reg.get("anthropic_backend.tools").is_some());
        assert!(reg.get("credential_store.tools").is_some());
    }

    #[test]
    fn register_team_registers_the_task_registry() {
        assert!(registry().get("task_registry.tools").is_some());
    }

    /// Boot the root and write its record without starting the kernel, so a
    /// test can seed a fresh tree and inspect it.
    fn boot_unstarted(dir: &Path) -> Arc<Kernel> {
        let mut booted = bootstrap::bootstrap(registry(), BootstrapOptions::one_shot(dir)).unwrap();
        lock::acquire(dir).unwrap();
        booted.write_seed().unwrap();
        booted.kernel.clone()
    }

    #[test]
    fn the_registry_has_fantastic_s_bundles() {
        let reg = registry();
        let names: Vec<&str> = reg.catalog().map(|(n, _)| n).collect();
        assert!(names.contains(&"file_bridge.tools"));
        assert!(names.contains(&"kernel_state.tools"));
    }

    #[test]
    fn the_registry_has_the_git_connector() {
        let reg = registry();
        let names: Vec<&str> = reg.catalog().map(|(n, _)| n).collect();
        assert!(names.contains(&"team_git.tools"));
    }

    #[test]
    fn the_registry_has_the_github_connector() {
        let reg = registry();
        let names: Vec<&str> = reg.catalog().map(|(n, _)| n).collect();
        assert!(names.contains(&"team_github.tools"));
    }

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn register_team_adds_the_team_bundles_to_any_registry() {
        let mut reg = BundleRegistry::new();
        register_team(&mut reg);
        let names: Vec<&str> = reg.catalog().map(|(n, _)| n).collect();
        assert!(names.contains(&"team_git.tools"));
        assert!(names.contains(&"team_github.tools"));
        // An `fantastic_tui::Extra` is a plain fn over the registry.
        let _extra: fantastic_tui::Extra = register_team;
    }

    #[test]
    fn register_team_registers_the_team_page_bundle() {
        let mut reg = BundleRegistry::new();
        register_team(&mut reg);
        let names: Vec<&str> = reg.catalog().map(|(n, _)| n).collect();
        assert!(names.contains(&"team_page.tools"));
    }

    #[test]
    fn register_team_registers_puller() {
        let mut reg = BundleRegistry::new();
        register_team(&mut reg);
        let names: Vec<&str> = reg.catalog().map(|(n, _)| n).collect();
        assert!(names.contains(&"team_puller.tools"));
    }

    #[test]
    fn fantastic_s_one_shot_commands_are_named() {
        assert!(is_one_shot(&v(&["mcp"])));
        assert!(is_one_shot(&v(&["send", "{}"])));
        assert!(is_one_shot(&v(&["readme"])));
        assert!(is_one_shot(&v(&["import", "dir", "--parent", "user"])));
        assert!(!is_one_shot(&[]));
        assert!(!is_one_shot(&v(&["--headless"])));
        assert!(!is_one_shot(&v(&["canvas", "--port", "1"])));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_fresh_dir_gets_a_base_tree() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = boot_unstarted(dir.path());
        assert!(seed_base_tree(&kernel).await.unwrap());
        let root = kernel
            .send(vec![Item::new(DEFAULT_ROOT_ID, json!({"type": "reflect"}))])
            .await;
        assert_eq!(
            root[0]
                .result
                .get("memory_store_id")
                .and_then(|v| v.as_str()),
            Some("kernel_memory")
        );
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_seeded_dir_is_left_alone() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = boot_unstarted(dir.path());
        assert!(seed_base_tree(&kernel).await.unwrap());
        assert!(!seed_base_tree(&kernel).await.unwrap());
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    /// Reflect an agent and return its record.
    async fn reflect_of(kernel: &Arc<Kernel>, id: &str) -> Value {
        let outcomes = kernel
            .send(vec![Item::new(id, json!({"type": "reflect"}))])
            .await;
        outcomes[0].result.clone()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn seeding_twice_is_a_no_op() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = boot_unstarted(dir.path());
        assert!(seed_base_tree(&kernel).await.unwrap());
        let before = reflect_of(&kernel, DEFAULT_ROOT_ID).await;
        assert!(seed_base_tree(&kernel).await.is_ok());
        let after = reflect_of(&kernel, DEFAULT_ROOT_ID).await;
        assert_eq!(before, after);
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_directory_seeded_by_the_shell_takes_the_base_tree() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(fantastic_tui::seed_dir(dir.path()).unwrap());
        let kernel = boot_unstarted(dir.path());
        assert!(seed_base_tree(&kernel).await.unwrap());
        let root = reflect_of(&kernel, DEFAULT_ROOT_ID).await;
        assert_eq!(root["memory_store_id"], "kernel_memory", "{root}");
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_reflect_is_reported_not_reseeded() {
        // A kernel with no root: the reflect is refused, so nothing is seeded.
        let kernel = Arc::new(Kernel::new());
        let err = seed_base_tree(&kernel).await.unwrap_err();
        assert!(
            err.starts_with("base tree: reflect failed"),
            "wrong error: {err}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_seeded_store_opens_only_for_the_kernel() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = boot_unstarted(dir.path());
        assert!(seed_base_tree(&kernel).await.unwrap());
        let store = reflect_of(&kernel, "store").await;
        assert_eq!(store["ingress_rule"], "allow_all_ingress");
        assert_eq!(store["sealed"], false);
        // Only the kernel and its persistence loader may call it (`$self` is
        // the substrate's own entry on every agent; no `*`, no `user`).
        let by = store["callable_by"].as_object().expect("callable_by");
        let mut keys: Vec<&str> = by.keys().map(|k| k.as_str()).collect();
        keys.sort();
        assert_eq!(
            keys,
            vec!["$self", "kernel", "kernel_state"],
            "callable_by: {by:?}"
        );
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[test]
    fn register_team_registry_has_sqlite_store() {
        assert!(registry().get("sqlite_store.tools").is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn any_agent_record_survives_a_restart() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = start(dir.path()).await.unwrap();
        for o in kernel
            .send(vec![Item::new(
                DEFAULT_ROOT_ID,
                json!({"type": "create_agent", "parent_id": DEFAULT_ROOT_ID,
                       "id": "plain", "handler_module": "ram_store.tools",
                       "custom_field": "kept"}),
            )])
            .await
        {
            assert!(matches!(o.status, Status::Completed), "{}", o.result);
        }
        kernel.flush_records().await;
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();

        let kernel = start(dir.path()).await.unwrap();
        let plain = kernel.resolve_target("plain").expect("agent restored");
        let kept = plain.meta.read().unwrap().get("custom_field").cloned();
        assert_eq!(kept, Some(json!("kept")));
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_seed_releases_the_lock() {
        let dir = tempfile::TempDir::new().unwrap();
        // A persisted `store` agent already exists, so the seed's own
        // `create_agent store` fails — after the lock is taken.
        let agent = dir.path().join(".fantastic/agents/store");
        std::fs::create_dir_all(&agent).unwrap();
        std::fs::write(
            agent.join("agent.yaml"),
            "id: store\nhandler_module: file_bridge.tools\nroot: .\n",
        )
        .unwrap();
        let r = start(dir.path()).await;
        let err = r.err().expect("start must fail").to_string();
        assert!(
            err.contains("base tree seeding failed"),
            "wrong failure: {err}"
        );
        assert!(lock::acquire(dir.path()).is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_headless_host_boots_and_answers_reflect() {
        let dir = tempfile::TempDir::new().unwrap();
        let kernel = start(dir.path()).await.unwrap();
        let outcomes = kernel
            .send(vec![Item::new(DEFAULT_ROOT_ID, json!({"type": "reflect"}))])
            .await;
        let first = &outcomes[0];
        assert!(matches!(first.status, Status::Completed));
        assert!(first.result.get("id").is_some());
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }
}
