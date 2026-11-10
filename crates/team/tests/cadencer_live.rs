//! Live test (#112): the cadencer's settings survive a restart of a real kernel. Boots the
//! team registry the way the binary does, creates a board and a cadencer.

use std::sync::Arc;

use fantastic_kernel::bootstrap::{self, DEFAULT_ROOT_ID};
use fantastic_kernel::{AgentId, Item, Kernel};
use serde_json::{json, Value};

async fn call(k: &Arc<Kernel>, to: &str, payload: Value) -> Value {
    k.send(vec![Item::new(AgentId::from(to), payload)])
        .await
        .into_iter()
        .next()
        .map(|o| o.result)
        .unwrap_or(Value::Null)
}

async fn create_cadencer_and_board(kernel: &Arc<Kernel>) {
    call(
        kernel,
        DEFAULT_ROOT_ID,
        json!({
            "type": "create_agent",
            "parent_id": DEFAULT_ROOT_ID,
            "id": "board",
            "handler_module": "team_board.tools",
            "github": "gh_absent",
            "store": "store"
        }),
    )
    .await;
    call(
        kernel,
        DEFAULT_ROOT_ID,
        json!({
            "type": "create_agent",
            "parent_id": DEFAULT_ROOT_ID,
            "id": "cad",
            "handler_module": "team_cadencer.tools"
        }),
    )
    .await;
}

async fn restart(dir: &std::path::Path, kernel: Arc<Kernel>) -> Arc<Kernel> {
    kernel.flush_records().await;
    kernel.shutdown().await;
    bootstrap::shutdown(dir).unwrap();
    fantastic_team::start(dir).await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let kernel = fantastic_team::start(dir.path()).await.unwrap();
    create_cadencer_and_board(&kernel).await;
    let set = call(
        &kernel,
        "cad",
        json!({"type": "set", "quiet": false, "cadence": 5}),
    )
    .await;
    assert!(set.get("error").is_none(), "set refused: {set}");

    let kernel = restart(dir.path(), kernel).await;
    let st = call(&kernel, "cad", json!({"type": "status"})).await;
    assert_eq!(st["settings"]["quiet"], false, "quiet not restored: {st}");
    assert_eq!(st["settings"]["cadence"], 5, "cadence not restored: {st}");

    kernel.shutdown().await;
    bootstrap::shutdown(dir.path()).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn configure_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let kernel = fantastic_team::start(dir.path()).await.unwrap();
    create_cadencer_and_board(&kernel).await;
    call(
        &kernel,
        "cad",
        json!({"type": "configure", "parent": "lead"}),
    )
    .await;

    let kernel = restart(dir.path(), kernel).await;

    // The record holds the parent under `cadencer_parent`, never `parent`.
    let yaml =
        std::fs::read_to_string(dir.path().join(".fantastic/agents/cad/agent.yaml")).unwrap();
    let rec: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(rec["cadencer_parent"], "lead", "record: {yaml}");
    assert_ne!(rec["parent"], "lead", "record: {yaml}");

    // The restored parent is still accepted as a sender.
    let set = call(
        &kernel,
        "cad",
        json!({"type": "set", "sender": "lead", "cadence": 9}),
    )
    .await;
    assert!(set.get("error").is_none(), "set from parent refused: {set}");

    kernel.shutdown().await;
    bootstrap::shutdown(dir.path()).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn off_list_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let kernel = fantastic_team::start(dir.path()).await.unwrap();
    create_cadencer_and_board(&kernel).await;
    let set = call(
        &kernel,
        "cad",
        json!({"type": "set", "lead": "g", "on": false}),
    )
    .await;
    assert!(set.get("error").is_none(), "set refused: {set}");

    let kernel = restart(dir.path(), kernel).await;
    let st = call(&kernel, "cad", json!({"type": "status"})).await;
    assert_eq!(
        st["settings"]["off"],
        json!(["g"]),
        "off not restored: {st}"
    );

    kernel.shutdown().await;
    bootstrap::shutdown(dir.path()).unwrap();
}
