//! #222: on a fresh home the app chooses fantastic's `plain` key mode, so fantastic's
//! credential store (`credentials`, made at boot only once a mode is chosen) exists for the
//! team's agents that name it; a mode the user chose stays. The team's agents are not made
//! by the app: the first `webui` run seeds them from `crates/team/blueprint.yaml` (#261).
//! Real kernels: the team binary runs `webui --port N` in a folder, with its own home.
#![cfg(unix)]

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use fantastic_team::new_team;
use fantastic_team::tree::ROOT_ID;
use serde_json::{json, Value};
use tokio::process::{Child, Command};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_fantastic_team")
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

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

    /// The app's one step for a new team: the key mode.
    async fn choose_key_mode(&self) -> Result<bool, String> {
        new_team::choose_key_mode(|t, p| self.send(t, p)).await
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

fn has(list: &Value, id: &str) -> bool {
    new_team::has_agent(list, id)
}

#[tokio::test]
async fn a_fresh_home_gets_plain_and_the_seeded_team_starts() {
    let site = Site::new();
    let kernel = site.start().await;
    // the first run seeded the team; the key mode is the one choice left
    for id in ["lead", "roles", "board", "github"] {
        assert!(has(&site.agents().await, id), "seeded: {id}");
    }
    // the bug: no mode, so no credential store for the team's agents to name
    assert!(!has(&site.agents().await, "credentials"));
    assert_eq!(site.mode().await, None);

    assert_eq!(site.choose_key_mode().await, Ok(true));
    assert_eq!(site.mode().await.as_deref(), Some("plain"));
    let agents = site.agents().await;
    for id in ["credentials", "lead", "roles", "board", "github"] {
        assert!(has(&agents, id), "{id}");
    }
    assert!(
        !has(&agents, new_team::KEY_MODE_AGENT),
        "the helper agent is gone"
    );
    kernel.stop().await;

    // the team starts again from what the folder and the home hold
    let again = site.start().await;
    let agents = site.agents().await;
    for id in ["credentials", "lead", "roles", "board", "github"] {
        assert!(has(&agents, id), "after a restart: {id}");
    }
    assert_eq!(site.mode().await.as_deref(), Some("plain"));
    again.stop().await;
}

#[tokio::test]
async fn a_home_with_secure_chosen_stays_secure() {
    let site = Site::new();
    // the user chose secure (the set-up page's call), and fantastic booted with it
    let first = site.start().await;
    site.send(
        ROOT_ID,
        json!({"type": "create_agent", "parent_id": "user", "id": "pick",
               "handler_module": "setup.tools"}),
    )
    .await
    .unwrap();
    let set = site
        .send("pick", json!({"type": "set_key_mode", "mode": "secure"}))
        .await
        .unwrap();
    assert_eq!(set["mode"], "secure", "{set}");
    site.send(ROOT_ID, json!({"type": "delete_agent", "id": "pick"}))
        .await
        .unwrap();
    first.stop().await;

    let kernel = site.start().await;
    assert!(has(&site.agents().await, "credentials"));
    assert_eq!(site.choose_key_mode().await, Ok(false));
    assert_eq!(site.mode().await.as_deref(), Some("secure"));
    assert!(has(&site.agents().await, "lead"));
    kernel.stop().await;
}
