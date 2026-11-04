//! #219: the example blueprint folders import through fantastic's own `import_subtree` (the
//! `user` agent), in a home seeded the way the CLI seeds one, with this repo's bundles.

use std::path::{Path, PathBuf};

use fantastic_kernel::Item;
use serde_json::json;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_worker_only_example_imports_and_the_agent_carries_its_fields() {
    let dir = tempfile::TempDir::new().unwrap();
    fantastic_tui::seed_dir(dir.path()).unwrap();
    let kernel = fantastic_team::start(dir.path()).await.unwrap();
    let mk = json!({"type": "create_agent", "parent_id": "user", "id": "credentials",
        "handler_module": "credential_store.tools", "backend": "env", "refs": {}});
    let o = kernel
        .send(vec![Item::new(
            fantastic_kernel::bootstrap::DEFAULT_ROOT_ID,
            mk,
        )])
        .await;
    assert!(o[0].result.get("error").is_none(), "{}", o[0].result);

    let path = root()
        .join("examples/blueprints/worker_only")
        .canonicalize()
        .unwrap();
    let item = |approve: bool| {
        let mut p = json!({"type": "import_subtree", "path": path.to_string_lossy(),
            "parent_id": "user",
            "bind": {"@memory": "memory", "@credentials": "credentials"}});
        if approve {
            p["approve"] = json!(true);
        }
        p
    };
    // without `approve` it only previews
    let preview = kernel.send(vec![Item::new("user", item(false))]).await;
    assert_eq!(
        preview[0].result["approval_required"], true,
        "{}",
        preview[0].result
    );
    assert!(!kernel
        .agents
        .contains_key(&fantastic_kernel::AgentId::from("w1_1")));
    let done = kernel.send(vec![Item::new("user", item(true))]).await;
    let reply = &done[0].result;
    assert!(reply.get("error").is_none(), "{reply}");
    let id = reply["ids"]["w1"].as_str().expect("the fresh id of w1");
    let meta = kernel
        .agents
        .get(&fantastic_kernel::AgentId::from(id))
        .map(|a| a.meta.read().unwrap().clone())
        .unwrap();
    assert_eq!(meta["model"], "example-model");
    assert_eq!(meta["memory_store_id"], "memory");
    assert_eq!(meta["credentials"]["store"], "credentials");
    kernel.shutdown().await;
    fantastic_sqlite_store::close_all(&kernel);
    fantastic_kernel::bootstrap::shutdown(dir.path()).unwrap();
}
