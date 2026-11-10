//! #261: the callee end of the team's edges on fantastic's stock agents comes from the team's
//! yaml alone (parent entries, fantastic#500), the team never chooses a key mode, and a boot
//! creates, updates and grants nothing. Real kernels: the team binary runs `webui --port N`
//! in a folder with its own home, and the person (this test) acts through `send`.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use fantastic_team::tree::ROOT_ID;
use serde_json::{json, Value};
use tokio::process::{Child, Command};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_fantastic_team")
}

mod common;
use common::free_port;

/// One folder with its own home, as the app has: fantastic keeps the key mode in the home.
struct Site {
    _tmp: tempfile::TempDir,
    dir: PathBuf,
    home: PathBuf,
}

impl Site {
    fn new() -> Site {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("team");
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        Site {
            _tmp: tmp,
            dir,
            home,
        }
    }

    fn command(&self) -> Command {
        let mut c = Command::new(bin());
        c.current_dir(&self.dir)
            .env("HOME", &self.home)
            .env_remove("FANTASTIC_HOME")
            .stdin(Stdio::null());
        c
    }

    /// `<binary> send` in the folder: the item's `result`.
    async fn send(&self, target: &str, payload: Value) -> Result<Value, String> {
        let item = json!({"target": target, "payload": payload}).to_string();
        let out = self
            .command()
            .arg("send")
            .arg(item)
            .output()
            .await
            .map_err(|e| e.to_string())?;
        let row: Value = serde_json::from_slice(&out.stdout).map_err(|e| {
            format!(
                "no JSON ({e}): {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )
        })?;
        if row["status"] != "completed" {
            return Err(format!("not completed: {row}"));
        }
        Ok(row["result"].clone())
    }

    /// Start the kernel (`webui --port N`, which seeds the team at the first run) and wait
    /// until it answers `send`.
    async fn start(&self) -> Kernel {
        let child = self
            .command()
            .args(["webui", "--port", &free_port().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut kernel = Kernel { child };
        let until = Instant::now() + Duration::from_secs(60);
        while self
            .send(ROOT_ID, json!({"type": "list_agents"}))
            .await
            .is_err()
        {
            assert!(Instant::now() < until, "the kernel never answered send");
            if let Some(s) = kernel.child.try_wait().unwrap() {
                panic!("the kernel exited early: {s}");
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        kernel
    }

    async fn agents(&self) -> Value {
        self.send(ROOT_ID, json!({"type": "list_agents"}))
            .await
            .unwrap()
    }

    /// The kernel's key mode as fantastic's set-up agent says it: `None` is not chosen.
    async fn mode(&self) -> Option<String> {
        let id = "probe_setup";
        self.send(
            ROOT_ID,
            json!({"type": "create_agent", "parent_id": "user", "id": id,
                   "handler_module": "setup.tools"}),
        )
        .await
        .unwrap();
        let m = self.send(id, json!({"type": "key_mode"})).await.unwrap();
        self.send(ROOT_ID, json!({"type": "delete_agent", "id": id}))
            .await
            .unwrap();
        m["mode"].as_str().map(str::to_string)
    }
}

/// How long a SIGTERM stop may take (the shell drains, the watchdog allows 4s).
const STOP_BOUND: Duration = Duration::from_secs(30);

struct Kernel {
    child: Child,
}

impl Kernel {
    /// Stop it as a person does: SIGTERM, then wait (bounded) for it to exit. The records
    /// reach the disk before it does; a hang past the bound fails the test.
    async fn stop(mut self) {
        let pid = self.child.id().expect("the kernel is running");
        let st = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .unwrap();
        assert!(st.success(), "kill -TERM {pid}");
        let exit = tokio::time::timeout(STOP_BOUND, self.child.wait())
            .await
            .unwrap_or_else(|_| panic!("the kernel was still running {STOP_BOUND:?} after SIGTERM"))
            .unwrap();
        assert_eq!(exit.code(), Some(0), "a SIGTERM stop exits cleanly: {exit}");
    }
}

impl Site {
    /// The record of agent `id` as the live kernel holds it.
    async fn record(&self, id: &str) -> Value {
        self.agents().await["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id)
            .unwrap_or_else(|| panic!("no agent {id}"))
            .clone()
    }

    fn ids_of(list: &Value) -> Vec<String> {
        let mut v: Vec<String> = list["agents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_string())
            .collect();
        v.sort();
        v
    }
}

fn has_grant(rec: &Value, field: &str, who: &str, want: Value) {
    assert_eq!(rec[field][who], want, "{}.{field}.{who}: {rec}", rec["id"]);
}

const PAGE_USER_VERBS: [&str; 2] = ["inquiries", "answer_inquiry"];

/// The callee-side grants the yaml declares, on the live records.
async fn assert_the_declared_grants(site: &Site) {
    let memory = site.record("memory").await;
    has_grant(&memory, "callable_by", "roles", json!("*"));
    // the stock grants stay next to them, and the lead's end (fantastic's own reach)
    has_grant(&memory, "callable_by", "user", json!(["*"]));
    has_grant(&memory, "callable_by", "lead", json!(["*"]));
    let user = site.record("user").await;
    // fantastic's own `send` is on the leg's entry too (reach); the page's verbs are next to it
    for verb in PAGE_USER_VERBS {
        assert!(
            user["callable_by"]["team_ws"]
                .as_array()
                .unwrap()
                .contains(&json!(verb)),
            "user admits team_ws {verb}: {user}"
        );
    }
    let root = site.record("kernel_state").await;
    has_grant(&root, "callable_by", "lead", json!(["*"]));
    has_grant(&root, "callable_by", "user", json!(["*"]));
}

/// A first run seeds the grants (the calls over them are `hydrate::tests`), a second boot keeps them,
/// and the boot created nothing the person did not: no key mode, no credential store, no
/// helper agent.
#[tokio::test]
async fn the_yaml_grants_hold_on_both_ends_over_a_real_call_and_a_second_boot_keeps_them() {
    let site = Site::new();
    let kernel = site.start().await;
    assert_the_declared_grants(&site).await;

    // a boot makes nothing of its own: no key mode, no store, no helper
    let ids = Site::ids_of(&site.agents().await);
    for id in ["credentials", "key_mode_setup", "setup", "probe_setup"] {
        assert!(!ids.contains(&id.to_string()), "{id} exists: {ids:?}");
    }
    assert_eq!(site.mode().await, None, "the team chose no key mode");
    kernel.stop().await;

    // a second boot keeps every grant, and creates nothing either
    let again = site.start().await;
    assert_the_declared_grants(&site).await;
    assert_eq!(Site::ids_of(&site.agents().await), ids);
    assert_eq!(site.mode().await, None);
    again.stop().await;
}
