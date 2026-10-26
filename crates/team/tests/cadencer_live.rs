//! Live test (#112): the cadencer delivers through a real kernel. Boots the
//! team registry the way the binary does, creates a board with a group, a
//! cadencer and a driver, and checks the timer beats and the board is read.

use std::sync::Arc;

use fantastic_kernel::bootstrap::{self, BootstrapOptions, DEFAULT_ROOT_ID};
use fantastic_kernel::bundle::{Bundle, BundleError, Reply};
use fantastic_kernel::{lock, AgentId, Item, Kernel};
use serde_json::{json, Value};

async fn call(k: &Arc<Kernel>, to: &str, payload: Value) -> Value {
    k.send(vec![Item::new(AgentId::from(to), payload)])
        .await
        .into_iter()
        .next()
        .map(|o| o.result)
        .unwrap_or(Value::Null)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_board_and_cadencer_deliver_through_a_real_kernel() {
    let dir = tempfile::tempdir().unwrap();
    let kernel = fantastic_team::start(dir.path()).await.unwrap();

    call(
        &kernel,
        DEFAULT_ROOT_ID,
        json!({
            "type": "create_agent",
            "parent_id": DEFAULT_ROOT_ID,
            "id": "board",
            "handler_module": "team_board.tools",
            "github": "gh_absent",
            "store": "store",
            "team_groups": {"rtx": {"driver": "rtx-driver", "members": ["rtx-w1", "rtx-w2"]}}
        }),
    )
    .await;
    call(
        &kernel,
        DEFAULT_ROOT_ID,
        json!({
            "type": "create_agent",
            "parent_id": DEFAULT_ROOT_ID,
            "id": "cad",
            "handler_module": "team_cadencer.tools"
        }),
    )
    .await;
    call(
        &kernel,
        "cad",
        json!({"type": "configure", "board": "board", "parent": "mcp"}),
    )
    .await;

    // Bug 1: the timer must run right after create + configure, with no set.
    let st = call(&kernel, "cad", json!({"type": "status"})).await;
    assert!(
        !st["next_beat_s"].is_null(),
        "timer not started after create: {st}"
    );

    // Bug 2: a set from the operator (no origin) must be accepted.
    let set = call(&kernel, "cad", json!({"type": "set", "cadence": 1})).await;
    assert!(set.get("error").is_none(), "set refused: {set}");

    // Bug 3: the timer beats, reads the board's rows and delivers a digest.
    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    let st = call(&kernel, "cad", json!({"type": "status"})).await;
    assert!(st["ticks"].as_u64().unwrap_or(0) >= 1, "no beat: {st}");
    assert!(
        st["digests_sent"].as_u64().unwrap_or(0) >= 1,
        "board rows not read: {st}"
    );

    kernel.shutdown().await;
    bootstrap::shutdown(dir.path()).unwrap();
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
            "store": "store",
            "team_groups": {"rtx": {"driver": "rtx-driver", "members": ["rtx-w1", "rtx-w2"]}}
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
        json!({"type": "configure", "board": "board", "parent": "lead"}),
    )
    .await;

    let kernel = restart(dir.path(), kernel).await;
    let st = call(&kernel, "cad", json!({"type": "status"})).await;
    assert!(
        !st["next_beat_s"].is_null(),
        "timer not restored at boot: {st}"
    );

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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn seeded_store_rule_survives_restart_from_the_seed() {
    let dir = tempfile::tempdir().unwrap();
    let kernel = fantastic_team::start(dir.path()).await.unwrap();
    let store = call(&kernel, "store", json!({"type": "reflect"})).await;
    assert_eq!(store["ingress_rule"], "allow_all_ingress", "live: {store}");

    // The record carries the seed's own rule, not one set by a later touch.
    let kernel = restart(dir.path(), kernel).await;
    let yaml =
        std::fs::read_to_string(dir.path().join(".fantastic/agents/store/agent.yaml")).unwrap();
    let rec: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(rec["ingress_rule"], "allow_all_ingress", "record: {yaml}");

    let store = call(&kernel, "store", json!({"type": "reflect"})).await;
    assert_eq!(
        store["ingress_rule"], "allow_all_ingress",
        "after restart: {store}"
    );
    assert_eq!(store["sealed"], false, "after restart: {store}");

    kernel.shutdown().await;
    bootstrap::shutdown(dir.path()).unwrap();
}

/// A scripted AI agent: reports 500 tokens of usage and accepts every send.
struct ScriptedDriver;

#[async_trait::async_trait]
impl Bundle for ScriptedDriver {
    fn name(&self) -> &str {
        "scripted_driver"
    }

    fn verb_docs(&self) -> Value {
        json!({})
    }

    async fn handle(
        &self,
        _agent_id: &AgentId,
        payload: &Value,
        _kernel: &Arc<Kernel>,
    ) -> Result<Reply, BundleError> {
        let t = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
        Ok(Some(match t {
            "reflect" => json!({"usage": {"prompt_tokens": 300, "output_tokens": 200}}),
            "send" => json!({"ok": true}),
            "status" => json!({"status": "idle"}),
            "rows" => json!({"rows": []}),
            _ => json!({}),
        }))
    }
}

async fn start_with_scripted_driver(dir: &std::path::Path) -> Arc<Kernel> {
    let mut reg = fantastic_team::registry();
    reg.register("scripted_driver.tools", ScriptedDriver);
    let mut booted = bootstrap::bootstrap(reg, BootstrapOptions::one_shot(dir)).unwrap();
    lock::acquire(dir).unwrap();
    let kernel = booted.kernel.clone();
    booted.write_seed().unwrap();
    fantastic_team::seed_base_tree(&kernel).await.unwrap();
    let started = kernel.start(true).await;
    assert!(started.get("error").is_none(), "start: {started}");
    kernel
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_status_shows_digest_bytes_and_driver_tokens_after_a_beat() {
    let dir = tempfile::tempdir().unwrap();
    let kernel = start_with_scripted_driver(dir.path()).await;
    for id in ["rtx-driver", "rtx-w1", "rtx-w2"] {
        call(
            &kernel,
            DEFAULT_ROOT_ID,
            json!({
                "type": "create_agent",
                "parent_id": DEFAULT_ROOT_ID,
                "id": id,
                "handler_module": "scripted_driver.tools",
                "callable_by": {"$self": "*", "board": "*"}
            }),
        )
        .await;
    }
    // The board reads the agents through the kernel, so it needs the egress grant.
    call(
        &kernel,
        DEFAULT_ROOT_ID,
        json!({
            "type": "create_agent",
            "parent_id": DEFAULT_ROOT_ID,
            "id": "board",
            "handler_module": "team_board.tools",
            "github": "gh_absent",
            "store": "store",
            "team_groups": {"rtx": {"driver": "rtx-driver", "members": ["rtx-w1", "rtx-w2"]}},
            "can_call": {"store": "*", "rtx-driver": "*", "rtx-w1": "*", "rtx-w2": "*"}
        }),
    )
    .await;
    call(
        &kernel,
        DEFAULT_ROOT_ID,
        json!({
            "type": "create_agent",
            "parent_id": DEFAULT_ROOT_ID,
            "id": "cad",
            "handler_module": "team_cadencer.tools"
        }),
    )
    .await;
    call(
        &kernel,
        "cad",
        json!({"type": "configure", "board": "board", "parent": "mcp"}),
    )
    .await;

    let st = call(&kernel, "cad", json!({"type": "status"})).await;
    assert_eq!(st["digest_bytes_per_hour"], 0, "before the beat: {st}");
    assert_eq!(st["driver_tokens_per_hour"], 0, "before the beat: {st}");

    let set = call(&kernel, "cad", json!({"type": "set", "cadence": 1})).await;
    assert!(set.get("error").is_none(), "set refused: {set}");

    let mut st = Value::Null;
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        st = call(&kernel, "cad", json!({"type": "status"})).await;
        if st["digests_sent"].as_u64().unwrap_or(0) >= 1
            && st["digest_bytes_per_hour"].as_u64().unwrap_or(0) > 0
            && st["driver_tokens_per_hour"].as_u64().unwrap_or(0) > 0
        {
            break;
        }
    }
    assert!(
        st["digest_bytes_per_hour"].as_u64().unwrap_or(0) > 0,
        "no digest bytes metered: {st}"
    );
    assert_eq!(
        st["driver_tokens_per_hour"], 500,
        "driver tokens only: {st}"
    );
    assert_eq!(st["load"]["tokens_total"], 500, "load: {st}");
    assert!(
        st["delivery_errors"].is_null() || st["delivery_errors"] == 0,
        "delivery errors: {st}"
    );

    kernel.shutdown().await;
    bootstrap::shutdown(dir.path()).unwrap();
}
