//! In-process control of the headless team: start, pause, resume, stop. These
//! are the same kernel messages any client sends; there is no UI-only path.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use fantastic_kernel::bootstrap::{self, DEFAULT_ROOT_ID};
use fantastic_kernel::{Item, Kernel, Status};
use serde_json::{json, Map, Value};

use crate::cadencer::HANDLER_MODULE;
use crate::{hydrate, page, start, start_offline, start_on_port};

/// Where the team is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Started and taking work.
    Running,
    /// Started, with the agents that can be paused told to hold their work.
    Paused,
    /// Shut down and the directory lock released; the handle holds no kernel.
    Stopped,
    /// The kernel is shut down but releasing the directory lock failed. The
    /// team is not claimed as stopped: a later [`TeamHandle::stop`] retries
    /// the release only.
    StopFailed,
}

/// Options for [`TeamHandle::start`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TeamOptions {
    /// A web UI port on 127.0.0.1. `Some(p)`: the handle serves the team page
    /// there (see [`TeamHandle::url`]) until it is stopped; a port that is taken is
    /// an error and leaves nothing running. `None` is headless.
    pub port: Option<u16>,
}

/// A running headless team in this process.
///
/// [`TeamHandle::stop`] is the clean path: it shuts the kernel down and returns
/// an error when the directory lock could not be released. Dropping a handle is
/// best effort: it shuts the kernel down on the current runtime when there is
/// one, releases the lock, and reports nothing.
pub struct TeamHandle {
    dir: PathBuf,
    kernel: Option<Arc<Kernel>>,
    state: State,
    /// Agents this handle paused, each with the message that undoes it.
    held: Vec<(String, Value)>,
    /// The port the web UI is served on, while it is.
    port: Option<u16>,
}

impl TeamHandle {
    /// Boot the team in `workdir` and start it. With `opts.port` it also serves
    /// the team page on that port; when that fails the kernel is shut down
    /// and the directory lock released before the error is returned.
    pub async fn start(workdir: &Path, opts: TeamOptions) -> Result<TeamHandle, String> {
        if opts.port.is_some() {
            // FIRST RUN of a web UI: the team's agents, from the blueprint (`hydrate::seed`).
            hydrate::seed(workdir)?;
        }
        let kernel = match opts.port {
            Some(port) => start_on_port(workdir, port).await,
            None => start(workdir).await,
        }
        .map_err(|e| e.to_string())?;
        if let Some(port) = opts.port {
            if let Err(e) = serve_web(&kernel, port).await {
                drop_web(&kernel).await;
                kernel.shutdown().await;
                fantastic_sqlite_store::close_all(&kernel);
                return match bootstrap::shutdown(workdir) {
                    Ok(()) => Err(e),
                    Err(r) => Err(format!("{e}; releasing the lock failed: {r}")),
                };
            }
        }
        Ok(TeamHandle {
            dir: workdir.to_path_buf(),
            kernel: Some(kernel),
            state: State::Running,
            held: Vec::new(),
            port: opts.port,
        })
    }

    /// Where the web UI is served, when the handle was started with a port.
    pub fn url(&self) -> Option<String> {
        self.port
            .map(|p| format!("http://127.0.0.1:{p}{}", page::PAGE_PATH))
    }

    /// Hold the team's work. Every agent that answers `pause` is sent it, and
    /// the cadencer that is running is switched off. The state becomes
    /// [`State::Paused`] only when every send is answered; a node that refuses
    /// is named in the error and the state stays `Running`. Pausing a paused
    /// team is `Ok`.
    pub async fn pause(&mut self) -> Result<(), String> {
        match self.state {
            State::Stopped | State::StopFailed => return Err("team is stopped".into()),
            State::Paused => return Ok(()),
            State::Running => {}
        }
        let errs = self.hold_all().await;
        if errs.is_empty() {
            self.state = State::Paused;
            Ok(())
        } else {
            Err(format!("pause failed on: {}", errs.join("; ")))
        }
    }

    /// Release the agents [`TeamHandle::pause`] held. A node that refuses stays
    /// held and is named in the error. Resuming a running team is `Ok`.
    pub async fn resume(&mut self) -> Result<(), String> {
        match self.state {
            State::Stopped | State::StopFailed => return Err("team is stopped".into()),
            State::Running | State::Paused => {}
        }
        let errs = self.release_all().await;
        if errs.is_empty() {
            self.state = State::Running;
            Ok(())
        } else {
            Err(format!("resume failed on: {}", errs.join("; ")))
        }
    }

    /// Shut the kernel down and release the directory lock. Calling it again
    /// after a successful stop does nothing; calling it after a failed release
    /// retries the release.
    pub async fn stop(&mut self) -> Result<(), String> {
        self.stop_with(|dir| bootstrap::shutdown(dir).map_err(|e| e.to_string()))
            .await
    }

    /// [`TeamHandle::stop`] with the lock release supplied by the caller.
    async fn stop_with<F>(&mut self, release: F) -> Result<(), String>
    where
        F: FnOnce(&Path) -> Result<(), String>,
    {
        if self.state == State::Stopped {
            return Ok(());
        }
        if let Some(kernel) = self.kernel.take() {
            if self.port.take().is_some() {
                // The host's record would otherwise bind the port at the next start.
                drop_web(&kernel).await;
            }
            kernel.shutdown().await;
            fantastic_sqlite_store::close_all(&kernel);
        }
        self.held.clear();
        match release(&self.dir) {
            Ok(()) => {
                self.state = State::Stopped;
                Ok(())
            }
            Err(e) => {
                self.state = State::StopFailed;
                Err(e)
            }
        }
    }

    /// The team's current state.
    pub fn state(&self) -> State {
        self.state
    }

    /// Send `reflect` to the root and return its reply.
    pub async fn reflect(&self) -> Result<Value, String> {
        let kernel = self.kernel.as_ref().ok_or("team is stopped")?;
        let outcomes = kernel
            .send(vec![Item::new(DEFAULT_ROOT_ID, json!({"type": "reflect"}))])
            .await;
        match outcomes.first() {
            Some(o) if matches!(o.status, Status::Completed) => Ok(o.result.clone()),
            Some(o) => Err(format!("reflect failed: {}", o.result)),
            None => Err("reflect failed: no reply".into()),
        }
    }

    /// Send each agent that can be held its hold message and record the undo.
    /// Returns one line per node that refused.
    async fn hold_all(&mut self) -> Vec<String> {
        let Some(kernel) = self.kernel.clone() else {
            return vec!["team is stopped".into()];
        };
        let mut errs = Vec::new();
        for id in kernel.agent_ids() {
            let Some(module) = kernel
                .agents
                .get(&id)
                .and_then(|a| a.handler_module.clone())
            else {
                continue;
            };
            let (hold, undo) = if module == HANDLER_MODULE {
                // Only a cadencer that is running is held; one the operator
                // turned off stays off after a resume.
                let st = match send_one(&kernel, &id.0, json!({"type": "status"})).await {
                    Ok(st) => st,
                    Err(e) => {
                        errs.push(format!("{}: {e}", id.0));
                        continue;
                    }
                };
                if st["settings"]["enabled"] != json!(true) {
                    continue;
                }
                (
                    json!({"type": "set", "enabled": false}),
                    json!({"type": "set", "enabled": true}),
                )
            } else if verb_is_documented(&kernel, &id.0, "pause").await {
                (json!({"type": "pause"}), json!({"type": "resume"}))
            } else {
                continue;
            };
            match send_one(&kernel, &id.0, hold).await {
                Ok(_) => self.held.push((id.0.clone(), undo)),
                Err(e) => errs.push(format!("{}: {e}", id.0)),
            }
        }
        errs
    }

    /// Send each held agent its undo message. Those that refuse stay held.
    async fn release_all(&mut self) -> Vec<String> {
        let Some(kernel) = self.kernel.clone() else {
            return vec!["team is stopped".into()];
        };
        let mut errs = Vec::new();
        let mut still = Vec::new();
        for (id, undo) in std::mem::take(&mut self.held) {
            if let Err(e) = send_one(&kernel, &id, undo.clone()).await {
                errs.push(format!("{id}: {e}"));
                still.push((id, undo));
            }
        }
        self.held = still;
        errs
    }
}

impl Drop for TeamHandle {
    fn drop(&mut self) {
        let dir = self.dir.clone();
        let Some(kernel) = self.kernel.take() else {
            if self.state == State::StopFailed {
                let _ = bootstrap::shutdown(&dir);
            }
            return;
        };
        match tokio::runtime::Handle::try_current() {
            Ok(rt) => {
                rt.spawn(async move {
                    kernel.shutdown().await;
                    fantastic_sqlite_store::close_all(&kernel);
                    let _ = bootstrap::shutdown(&dir);
                });
            }
            Err(_) => {
                drop(kernel);
                let _ = bootstrap::shutdown(&dir);
            }
        }
    }
}

/// Serve the team page on `127.0.0.1:port`: the `web` host the blueprint seeded already
/// has the run's `--port` ([`start_on_port`]); it boots (it mounts the children, then
/// listens; idempotent once the kernel's start booted it). A taken port makes the boot fail.
async fn serve_web(kernel: &Arc<Kernel>, port: u16) -> Result<(), String> {
    send_one(kernel, page::WEB_ID, json!({"type": "boot"}))
        .await
        .map(|_| ())
        .map_err(|e| format!("serving the web UI on port {port} failed: {e}"))
}

/// Put the run's own flags on the `web` host the blueprint seeded: its `port` (`--port`,
/// else the one recorded, else a free one) and its address (`--host`). They are the run's,
/// not the blueprint's, so they cannot be declared; fantastic does the same for the `web`
/// of its own `canvas` word (`kern/host.rs`, `set_web_field` and `set_web_bind`, called by
/// `Host::boot` for `--port` and `--host`): a value the record already holds is left
/// alone. Nothing else is created, updated or granted: the host, the page and the leg, and
/// the edges between them, are the blueprint's. A host that records an address binds it:
/// another is refused, as fantastic's canvas refuses it (`kern/web.rs`, `ensure`).
/// Returns the port and the address the host binds.
pub(crate) async fn set_run_flags(
    kernel: &Arc<Kernel>,
    port: Option<u16>,
    bind: Option<&str>,
) -> Result<(u16, String), String> {
    if !kernel
        .agents
        .contains_key(&fantastic_kernel::AgentId::from(page::WEB_ID))
    {
        return Err(format!(
            "no `{}` host in this folder: the team's first run seeds it (`webui` in a folder fantastic has not run in)",
            page::WEB_ID
        ));
    }
    let rec = web_record_of(kernel);
    let recorded = rec.get("port").and_then(Value::as_u64).map(|p| p as u16);
    let mut set = Map::new();
    let host = match recorded {
        // A host that records a port was made with its address: another is refused.
        Some(_) => {
            let bound = fantastic_web::listen_host(&rec);
            if let Some(b) = bind.filter(|b| *b != bound) {
                return Err(format!("--host {b}: the `web` leg still binds {bound}"));
            }
            bound
        }
        // The seeded host has no port yet: the address is the run's. The fields
        // fantastic's canvas writes for `--host`: the listener's address, and the guard's
        // identity too unless it is a wildcard.
        None => {
            if let Some(b) = bind {
                set.insert("bind".into(), json!(b));
                if !is_wildcard(b) {
                    set.insert("host".into(), json!(b));
                }
            }
            bind.unwrap_or(fantastic_web::DEFAULT_HOST).to_string()
        }
    };
    let port = match (port, recorded) {
        (Some(p), _) | (None, Some(p)) => p,
        (None, None) => free_port(&host)?,
    };
    if recorded != Some(port) {
        // The host binds in a task of its own, so a taken port would only be logged.
        std::net::TcpListener::bind((host.as_str(), port))
            .map_err(|e| format!("serving the web UI on port {port} failed: {e}"))?;
        set.insert("port".into(), json!(port));
    }
    if !set.is_empty() {
        let mut upd = json!({"type": "update_agent", "id": page::WEB_ID});
        for (k, v) in set {
            upd[k] = v;
        }
        send_one(kernel, DEFAULT_ROOT_ID, upd).await?;
    }
    kernel.flush_records().await;
    Ok((port, host))
}

/// The live `web` host's record fields.
fn web_record_of(kernel: &Arc<Kernel>) -> serde_json::Map<String, Value> {
    kernel
        .agents
        .get(&fantastic_kernel::AgentId::from(page::WEB_ID))
        .and_then(|a| a.meta.read().ok().map(|m| m.clone()))
        .unwrap_or_default()
}

/// A wildcard listener address: a browser cannot dial it.
pub fn is_wildcard(host: &str) -> bool {
    matches!(host, "0.0.0.0" | "::")
}

/// A port free on `host` now (the OS picks it), for a first run that names none.
pub fn free_port(host: &str) -> Result<u16, String> {
    std::net::TcpListener::bind((host, 0))
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| format!("no free port to pick: {e}"))
}

/// Seed the team's records into `dir` and put the run's `--port` and `--host` on the web
/// host, for a host that boots the directory itself: the binary's `webui` hands the
/// records to fantastic's shell, which hydrates and boots them. The records are the
/// blueprint's (`hydrate::seed` at the first run, after fantastic's stock tree: the web
/// host, the page and the socket leg, the fixed agents, their grants and config, all from
/// `crates/team/blueprint.yaml`); a directory that already holds a kernel is not seeded
/// again and nothing of it is re-made, re-set or re-granted, as fantastic treats its own.
/// What is written here is the run's flags ([`set_run_flags`]), by an offline kernel
/// ([`start_offline`]: nothing in it listens, so the port answers only once the shell holds
/// the lock and serves it) on a runtime of its own; the directory lock is released before it
/// returns, on failure too.
///
/// `port` None keeps the recorded port, else picks a free one. `bind` is `--host`: a
/// fresh host binds it, a recorded one must already bind it. Returns the port and the
/// address the host binds.
pub fn persist_web(
    dir: &Path,
    port: Option<u16>,
    bind: Option<&str>,
) -> Result<(u16, String), String> {
    persist_web_then(dir, port, bind, |_, _| ()).map(|(served, ())| served)
}

/// [`persist_web`], with `look` run on the kernel that writes the records once they
/// are made, while it is still up (its answer is returned beside the port and the
/// address): a test sees there what that kernel serves.
fn persist_web_then<T>(
    dir: &Path,
    port: Option<u16>,
    bind: Option<&str>,
    look: impl FnOnce(&Arc<Kernel>, &(u16, String)) -> T,
) -> Result<((u16, String), T), String> {
    hydrate::seed(dir)?;
    let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    rt.block_on(async {
        let kernel = start_offline(dir).await.map_err(|e| e.to_string())?;
        let res = set_run_flags(&kernel, port, bind).await.map(|served| {
            let seen = look(&kernel, &served);
            (served, seen)
        });
        kernel.shutdown().await;
        fantastic_sqlite_store::close_all(&kernel);
        let released = bootstrap::shutdown(dir).map_err(|e| e.to_string());
        match (res, released) {
            (Ok(served), r) => r.map(|()| served),
            (Err(e), Ok(())) => Err(e),
            (Err(e), Err(r)) => Err(format!("{e}; releasing the lock failed: {r}")),
        }
    })
}

/// Remove the web host and its children (the page and the socket leg), so the
/// port is closed and no record is left to bind it again. Best effort.
async fn drop_web(kernel: &Arc<Kernel>) {
    if kernel
        .agents
        .contains_key(&fantastic_kernel::AgentId::from(page::WEB_ID))
    {
        let _ = send_one(
            kernel,
            DEFAULT_ROOT_ID,
            json!({"type": "delete_agent", "id": page::WEB_ID}),
        )
        .await;
        kernel.flush_records().await;
    }
}

/// Send one payload to an agent. A refusal (a non-completed outcome or an
/// `error` in the reply) is the `Err`.
async fn send_one(kernel: &Arc<Kernel>, to: &str, payload: Value) -> Result<Value, String> {
    let outcome = kernel
        .send(vec![Item::new(to, payload)])
        .await
        .into_iter()
        .next();
    match outcome {
        Some(o) if matches!(o.status, Status::Completed) && o.result.get("error").is_none() => {
            Ok(o.result)
        }
        Some(o) => Err(o.result.to_string()),
        None => Err("no reply".into()),
    }
}

/// Does the agent document `verb`? Asked of the root with `docs`.
async fn verb_is_documented(kernel: &Arc<Kernel>, id: &str, verb: &str) -> bool {
    match send_one(kernel, DEFAULT_ROOT_ID, json!({"type": "docs", "of": id})).await {
        Ok(docs) => docs["verbs"].get(verb).is_some(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn start_then_stop_leaves_no_running_task() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut h = TeamHandle::start(dir.path(), TeamOptions::default())
            .await
            .unwrap();
        assert_eq!(h.state(), State::Running);
        h.stop().await.unwrap();
        assert_eq!(h.state(), State::Stopped);
        assert!(h.kernel.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pause_then_resume_changes_state() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut h = TeamHandle::start(dir.path(), TeamOptions::default())
            .await
            .unwrap();
        h.pause().await.unwrap();
        assert_eq!(h.state(), State::Paused);
        h.resume().await.unwrap();
        assert_eq!(h.state(), State::Running);
        h.stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stop_twice_is_ok() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut h = TeamHandle::start(dir.path(), TeamOptions::default())
            .await
            .unwrap();
        h.stop().await.unwrap();
        h.stop().await.unwrap();
        assert_eq!(h.state(), State::Stopped);
        assert!(h.pause().await.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn two_handles_in_two_workdirs_run_side_by_side() {
        let a_dir = tempfile::TempDir::new().unwrap();
        let b_dir = tempfile::TempDir::new().unwrap();
        let mut a = TeamHandle::start(a_dir.path(), TeamOptions::default())
            .await
            .unwrap();
        let mut b = TeamHandle::start(b_dir.path(), TeamOptions::default())
            .await
            .unwrap();
        assert!(a.reflect().await.unwrap().get("id").is_some());
        assert!(b.reflect().await.unwrap().get("id").is_some());
        a.stop().await.unwrap();
        assert_eq!(b.state(), State::Running);
        b.stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_started_team_answers_reflect() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut h = TeamHandle::start(dir.path(), TeamOptions::default())
            .await
            .unwrap();
        let reply = h.reflect().await.unwrap();
        assert!(reply.get("id").is_some());
        h.stop().await.unwrap();
    }

    /// Create a cadencer agent under the root, the way the team boot does.
    async fn make_cadencer(h: &TeamHandle, id: &str) {
        let kernel = h.kernel.as_ref().expect("running");
        kernel
            .send(vec![Item::new(
                DEFAULT_ROOT_ID,
                json!({
                    "type": "create_agent",
                    "parent_id": DEFAULT_ROOT_ID,
                    "id": id,
                    "handler_module": crate::cadencer::HANDLER_MODULE,
                }),
            )])
            .await;
    }

    /// Send one payload to an agent and return its result.
    async fn ask(h: &TeamHandle, to: &str, payload: Value) -> Value {
        let kernel = h.kernel.as_ref().expect("running");
        kernel
            .send(vec![Item::new(to, payload)])
            .await
            .into_iter()
            .next()
            .map(|o| o.result)
            .unwrap_or(Value::Null)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pause_holds_the_cadencer_and_resume_releases_it() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut h = TeamHandle::start(dir.path(), TeamOptions::default())
            .await
            .unwrap();
        make_cadencer(&h, "cad").await;
        let st = ask(&h, "cad", json!({"type": "status"})).await;
        assert_eq!(st["settings"]["enabled"], json!(true), "{st}");
        h.pause().await.unwrap();
        let st = ask(&h, "cad", json!({"type": "status"})).await;
        assert_eq!(
            st["settings"]["enabled"],
            json!(false),
            "pause did not hold: {st}"
        );
        h.resume().await.unwrap();
        let st = ask(&h, "cad", json!({"type": "status"})).await;
        assert_eq!(
            st["settings"]["enabled"],
            json!(true),
            "resume did not release: {st}"
        );
        h.stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn paused_team_does_nothing() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut h = TeamHandle::start(dir.path(), TeamOptions::default())
            .await
            .unwrap();
        make_cadencer(&h, "cad").await;
        ask(&h, "cad", json!({"type": "set", "cadence": 1})).await;
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        let before = ask(&h, "cad", json!({"type": "status"})).await;
        assert!(
            before["ticks"].as_u64().unwrap_or(0) >= 1,
            "timer never beat: {before}"
        );
        h.pause().await.unwrap();
        let paused = ask(&h, "cad", json!({"type": "status"})).await;
        tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
        let after = ask(&h, "cad", json!({"type": "status"})).await;
        assert_eq!(
            paused["ticks"], after["ticks"],
            "a paused team still beat: {after}"
        );
        h.stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pausing_twice_is_ok_and_resume_on_running_is_ok() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut h = TeamHandle::start(dir.path(), TeamOptions::default())
            .await
            .unwrap();
        h.resume().await.unwrap();
        h.pause().await.unwrap();
        h.pause().await.unwrap();
        assert_eq!(h.state(), State::Paused);
        h.stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dropping_a_handle_releases_the_lock() {
        let dir = tempfile::TempDir::new().unwrap();
        let h = TeamHandle::start(dir.path(), TeamOptions::default())
            .await
            .unwrap();
        drop(h);
        let mut freed = None;
        for _ in 0..40 {
            match TeamHandle::start(dir.path(), TeamOptions::default()).await {
                Ok(h2) => {
                    freed = Some(h2);
                    break;
                }
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
            }
        }
        let mut h2 = freed.expect("lock still held after the handle was dropped");
        h2.stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stop_reports_a_failed_release_and_does_not_claim_stopped() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut h = TeamHandle::start(dir.path(), TeamOptions::default())
            .await
            .unwrap();
        let r = h.stop_with(|_| Err("release refused".to_string())).await;
        assert!(r.is_err(), "a failed release must be an error");
        assert_eq!(h.state(), State::StopFailed);
        assert_ne!(h.state(), State::Stopped);
        assert!(h.pause().await.is_err());
        h.stop().await.unwrap();
        assert_eq!(h.state(), State::Stopped);
    }

    /// fantastic's web host keeps its live servers in a process-wide map keyed by the
    /// agent id (`web`), so two tests that serve a web host at once undo each other.
    /// Every test that serves one holds this for its whole run.
    use crate::WEB_LOCK as WEB;

    /// A port that is free right now (bound, read, released).
    fn free_port() -> u16 {
        // Never the same port twice in this process: the tests run in parallel and a
        // port released here may be handed out again before its test binds it.
        static ISSUED: std::sync::Mutex<Vec<u16>> = std::sync::Mutex::new(Vec::new());
        loop {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = l.local_addr().unwrap().port();
            let mut issued = ISSUED.lock().unwrap();
            if !issued.contains(&port) {
                issued.push(port);
                return port;
            }
        }
    }

    /// One HTTP/1.0 GET on 127.0.0.1:port; the whole response as text.
    async fn http_get(port: u16, path: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut conn = None;
        for _ in 0..50 {
            match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                Ok(c) => {
                    conn = Some(c);
                    break;
                }
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(40)).await,
            }
        }
        let mut c = conn.expect("the port never opened");
        let req = format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n");
        c.write_all(req.as_bytes()).await.unwrap();
        let mut out = Vec::new();
        c.read_to_end(&mut out).await.unwrap();
        String::from_utf8_lossy(&out).into_owned()
    }

    fn status_of(resp: &str) -> u16 {
        assert!(resp.starts_with("HTTP/1."), "{resp}");
        resp.split_whitespace().nth(1).unwrap().parse().unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn start_with_a_port_serves_the_web_ui() {
        let _web = WEB.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        let mut h = TeamHandle::start(dir.path(), TeamOptions { port: Some(port) })
            .await
            .unwrap();
        assert_eq!(
            h.url().unwrap(),
            format!("http://127.0.0.1:{port}/team_page/file/index.html")
        );
        let resp = http_get(port, page::PAGE_PATH).await;
        let status = status_of(&resp);
        assert!(status == 200 || (300..400).contains(&status), "{resp}");
        assert_eq!(status, 200, "{resp}");
        let js = http_get(port, "/team_page/file/page.js").await;
        assert!(
            js.contains("function renderSidebar("),
            "the team page is served"
        );
        let shim = http_get(port, "/team_page/file/shim.js").await;
        assert!(
            shim.contains("window.fantastic = {"),
            "with the socket shim"
        );
        assert!(resp.contains("src=\"shim.js\""), "the page loads the shim");
        // A file the page does not serve is a 404.
        assert_eq!(
            status_of(&http_get(port, "/team_page/file/other.txt").await),
            404
        );
        h.stop().await.unwrap();
        assert_eq!(h.state(), State::Stopped);
        // Stopped: the port is closed and the host's record is gone.
        assert!(tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_err());
        let mut again = TeamHandle::start(dir.path(), TeamOptions::default())
            .await
            .unwrap();
        assert!(again.url().is_none());
        assert!(!again
            .kernel
            .as_ref()
            .unwrap()
            .agents
            .contains_key(&fantastic_kernel::AgentId::from(page::WEB_ID)));
        again.stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn start_with_a_busy_port_fails_and_releases_the_lock() {
        let _web = WEB.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        let busy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = busy.local_addr().unwrap().port();
        let err = TeamHandle::start(dir.path(), TeamOptions { port: Some(port) })
            .await
            .err()
            .expect("a busy port is an error");
        assert!(err.contains(&port.to_string()), "{err}");
        // The lock is free again: a headless start works.
        let mut h = TeamHandle::start(dir.path(), TeamOptions::default())
            .await
            .expect("the lock was released");
        h.stop().await.unwrap();
        drop(busy);
    }

    /// Open a raw websocket on `/team_ws/ws`; the handshake head and the stream.
    async fn ws_open(port: u16) -> (String, tokio::net::TcpStream) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut c = loop {
            match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                Ok(c) => break c,
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(40)).await,
            }
        };
        let hs = format!(
            "GET /team_ws/ws HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        c.write_all(hs.as_bytes()).await.unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            let mut b = [0u8; 1];
            c.read_exact(&mut b).await.unwrap();
            head.push(b[0]);
        }
        (String::from_utf8_lossy(&head).into_owned(), c)
    }

    /// A websocket handshake, one `call` frame for `target`, and the `reply` frame.
    async fn ws_call(port: u16, target: &str, payload: Value) -> Value {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (head, mut c) = ws_open(port).await;
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");
        let msg =
            json!({"type": "call", "target": target, "payload": payload, "id": "p1"}).to_string();
        assert!(msg.len() < 126);
        let mut frame = vec![0x81, 0x80 | msg.len() as u8, 0, 0, 0, 0];
        frame.extend_from_slice(msg.as_bytes());
        c.write_all(&frame).await.unwrap();
        // The leg also announces what it handles as `event` frames: read to the reply.
        loop {
            let mut h2 = [0u8; 2];
            c.read_exact(&mut h2).await.unwrap();
            let len = match h2[1] & 0x7f {
                126 => {
                    let mut l = [0u8; 2];
                    c.read_exact(&mut l).await.unwrap();
                    u16::from_be_bytes(l) as usize
                }
                127 => {
                    let mut l = [0u8; 8];
                    c.read_exact(&mut l).await.unwrap();
                    u64::from_be_bytes(l) as usize
                }
                n => n as usize,
            };
            let mut body = vec![0u8; len];
            c.read_exact(&mut body).await.unwrap();
            let frame: Value = serde_json::from_slice(&body).unwrap();
            if frame["type"] != "event" {
                return frame;
            }
        }
    }

    /// `persist_web` blocks on a runtime of its own, as the binary's main does: run it
    /// off the test's runtime.
    async fn persist(dir: &Path, port: u16) -> Result<(), String> {
        let dir = dir.to_path_buf();
        std::thread::spawn(move || persist_web(&dir, Some(port), None).map(|_| ()))
            .join()
            .unwrap()
    }

    /// The dispatcher, made with its own end of the page leg's edge (`callable_by`): the
    /// blueprint holds the leg's end (`team_ws.can_call`), the agent the person makes holds
    /// the other.
    async fn make_dispatcher(k: &Arc<Kernel>, groups: Value) {
        let mk = json!({"type": "create_agent", "id": "dispatcher", "parent_id": DEFAULT_ROOT_ID,
            "handler_module": crate::dispatcher::HANDLER_MODULE, "groups": groups,
            "callable_by": {page::WS_ID: ["sidebar", "tree", "open", "control"]}});
        send_one(k, DEFAULT_ROOT_ID, mk).await.unwrap();
    }

    /// The page's socket leg reaches the dispatcher: a raw websocket handshake and one
    /// `call` frame for `sidebar`, answered by a `reply` frame with the same id.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_page_socket_reaches_the_dispatcher() {
        let _web = WEB.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        // The first run seeded the host, the page and the leg; the dispatcher exists before
        // the host is served, as after a first start.
        hydrate::seed(dir.path()).unwrap();
        let k = start_on_port(dir.path(), port).await.unwrap();
        make_dispatcher(&k, json!({})).await;
        serve_web(&k, port).await.unwrap();
        let reply = ws_call(port, "dispatcher", json!({"type": "sidebar"})).await;
        assert_eq!(reply["type"], "reply", "{reply}");
        assert_eq!(reply["id"], "p1");
        assert!(reply["data"].get("error").is_none(), "{reply}");
        assert_eq!(reply["data"]["groups"], json!([]), "{reply}");
        drop_web(&k).await;
        k.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    /// `--port` in the binary: the records are written offline, then a plain start
    /// (what fantastic's host does) serves the page.
    #[tokio::test(flavor = "multi_thread")]
    async fn persist_web_then_a_plain_start_serves_the_page() {
        let _web = WEB.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        persist(dir.path(), port).await.unwrap();
        let k = start(dir.path()).await.unwrap();
        let resp = http_get(port, page::PAGE_PATH).await;
        assert_eq!(status_of(&resp), 200, "{resp}");
        let js = http_get(port, "/team_page/file/page.js").await;
        assert!(js.contains("function renderSidebar("), "the page is served");
        let shim = http_get(port, "/team_page/file/shim.js").await;
        assert!(
            shim.contains("window.fantastic = {"),
            "with the socket shim"
        );
        assert!(resp.contains("src=\"shim.js\""), "the page loads the shim");
        assert_eq!(
            status_of(&http_get(port, "/team_page/file/other.txt").await),
            404
        );
        k.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    /// The shell's first-run records: the `user` and, under it, the blueprint's store.
    const SEEDED_USER: &str = ".fantastic/agents/user/agent.yaml";
    const SEEDED_STORE: &str = ".fantastic/agents/user/agents/store/agent.yaml";

    #[tokio::test(flavor = "multi_thread")]
    async fn persist_web_seeds_a_fresh_directory() {
        let _web = WEB.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        persist(dir.path(), free_port()).await.unwrap();
        assert!(dir.path().join(SEEDED_USER).is_file(), "the user record");
        assert!(dir.path().join(SEEDED_STORE).is_file(), "the blueprint");
        bootstrap::shutdown(dir.path()).ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_fresh_directory_serves_team_page_on_the_first_start() {
        let _web = WEB.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        persist(dir.path(), port).await.unwrap();
        let k = start(dir.path()).await.unwrap();
        let resp = http_get(port, page::PAGE_PATH).await;
        assert_eq!(status_of(&resp), 200, "{resp}");
        let js = http_get(port, "/team_page/file/page.js").await;
        assert!(js.contains("function renderSidebar("), "{js}");
        assert_eq!(
            status_of(&http_get(port, "/team_page/file/other.txt").await),
            404
        );
        assert!(
            dir.path().join(SEEDED_STORE).is_file(),
            "the seed was not taken away"
        );
        k.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    /// A folder fantastic seeded with its stock tree only has no team and no `web` host.
    /// `webui` does not make them (that would be an ensure at a boot: fantastic never
    /// reads its blueprint again, `src/tui/src/boot/seed.rs`), it says so and writes nothing.
    #[tokio::test(flavor = "multi_thread")]
    async fn persist_web_changes_nothing_in_a_folder_the_team_did_not_seed() {
        let _web = WEB.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        assert!(fantastic_tui::seed_dir(dir.path()).unwrap());
        // one plain boot first: the kernel normalises the records it loads
        settle(dir.path()).await;
        let before = tree_of(dir.path());
        let err = persist(dir.path(), free_port()).await.unwrap_err();
        assert!(err.contains("no `web` host"), "{err}");
        assert_eq!(tree_of(dir.path()), before, "nothing was made or re-set");
    }

    /// One boot of the folder by a kernel that serves nothing, then its stop: the kernel
    /// writes the records it loads back in its own normal form.
    async fn settle(dir: &Path) {
        let k = start_offline(dir).await.unwrap();
        k.flush_records().await;
        k.shutdown().await;
        fantastic_sqlite_store::close_all(&k);
        bootstrap::shutdown(dir).unwrap();
    }

    /// Every agent record under `dir/.fantastic/agents`, by relative path.
    fn tree_of(dir: &Path) -> std::collections::BTreeMap<String, String> {
        fn walk(at: &Path, root: &Path, out: &mut std::collections::BTreeMap<String, String>) {
            for e in std::fs::read_dir(at).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, root, out);
                } else if p.file_name().is_some_and(|n| n == "agent.yaml") {
                    out.insert(
                        p.strip_prefix(root).unwrap().to_string_lossy().into_owned(),
                        std::fs::read_to_string(&p).unwrap(),
                    );
                }
            }
        }
        let root = dir.join(".fantastic/agents");
        let mut out = Default::default();
        walk(&root, &root, &mut out);
        out
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn persist_web_socket_reaches_the_dispatcher() {
        let _web = WEB.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        persist(dir.path(), port).await.unwrap();
        let k = start(dir.path()).await.unwrap();
        make_dispatcher(&k, json!({})).await;
        let (head, _c) = ws_open(port).await;
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");
        let reply = ws_call(port, "dispatcher", json!({"type": "sidebar"})).await;
        assert_eq!(reply["type"], "reply", "{reply}");
        assert_eq!(reply["id"], "p1");
        assert_eq!(reply["data"]["groups"], json!([]), "{reply}");
        k.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    /// The page leg's end of every edge is the blueprint's: the seeded `team_ws` may call
    /// the dispatcher's page verbs and each verb the page sends to the other tool agents
    /// (`web/team.html`), nothing else and never `*`. The other end is on the agent the
    /// person makes (its own `callable_by`).
    #[tokio::test(flavor = "multi_thread")]
    async fn the_seeded_page_leg_may_call_exactly_the_verbs_the_team_page_sends() {
        let dir = tempfile::TempDir::new().unwrap();
        hydrate::seed(dir.path()).unwrap();
        let cc = read_can_call(dir.path());
        let sends = page_sends();
        for t in ["cadencer", "user"] {
            assert!(sends.iter().any(|(a, _)| a == t), "page sends to {t}");
        }
        for (target, verb) in sends.iter().filter(|(t, _)| t != "dispatcher") {
            let may = |v: &Value| v.as_array().is_some_and(|a| a.contains(&json!(verb)));
            assert!(may(&cc[target]), "can_call {target} {verb}: {cc}");
            assert_ne!(cc[target], json!("*"));
        }
        assert_eq!(
            cc["dispatcher"],
            json!(["sidebar", "tree", "open", "control"])
        );
        for k in cc.as_object().unwrap().keys() {
            if k != "$self" && k != "dispatcher" {
                assert!(
                    sends.iter().any(|(t, _)| t == k),
                    "the seeded leg may call {k}, which the page never sends to: {cc}"
                );
            }
        }
        for id in ["kernel", "kernel_state", "queue", "tasks"] {
            assert!(cc.get(id).is_none(), "{id}: {cc}");
        }
    }

    /// Every `(target, verb)` the page sends through `askNode` with a literal id and verb.
    fn page_sends() -> Vec<(String, String)> {
        let re = regex::Regex::new(r#"askNode\('([a-z_]+)',\s*\{\s*type:\s*'([a-z_]+)'"#).unwrap();
        re.captures_iter(include_str!("../web/team.html"))
            .map(|c| (c[1].to_string(), c[2].to_string()))
            .collect()
    }

    /// The seeded `team_ws` record's `can_call`, read from disk.
    fn read_can_call(dir: &Path) -> Value {
        let text =
            std::fs::read_to_string(dir.join(".fantastic/agents/web/agents/team_ws/agent.yaml"))
                .unwrap();
        let rec: Value = serde_yaml::from_str(&text).unwrap();
        rec["can_call"].clone()
    }

    /// The run's flags are the one thing `webui` writes, and they are written once: the same
    /// flags again change no record of the folder.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_run_flags_are_set_once_and_nothing_else_is_written() {
        let _web = WEB.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        let seeded = {
            hydrate::seed(dir.path()).unwrap();
            settle(dir.path()).await;
            tree_of(dir.path())
        };
        persist(dir.path(), port).await.unwrap();
        let first = tree_of(dir.path());
        let changed: Vec<&String> = first
            .iter()
            .filter(|(k, v)| seeded.get(*k) != Some(*v))
            .map(|(k, _)| k)
            .collect();
        assert_eq!(
            changed,
            ["web/agent.yaml"],
            "only the host's record takes the port"
        );
        persist(dir.path(), port).await.unwrap();
        assert_eq!(tree_of(dir.path()), first, "the same flags change nothing");
    }

    /// `persist_web` with a bind, off the test's runtime (see [`persist`]); the port and
    /// the address the host binds.
    async fn persist_bound(
        dir: &Path,
        port: Option<u16>,
        bind: Option<&'static str>,
    ) -> Result<(u16, String), String> {
        let dir = dir.to_path_buf();
        std::thread::spawn(move || persist_web(&dir, port, bind))
            .join()
            .unwrap()
    }

    /// The record `persist_web` wrote for the `web` host, read from disk.
    fn web_yaml(dir: &Path) -> String {
        std::fs::read_to_string(dir.join(".fantastic/agents/web/agent.yaml")).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn persist_web_names_the_bind_the_way_fantastic_s_canvas_does() {
        let _web = WEB.lock().await;
        // a concrete address is the listener and the guard's identity; a wildcard only the listener
        let dir = tempfile::TempDir::new().unwrap();
        let (port, host) = persist_bound(dir.path(), Some(free_port()), Some("127.0.0.1"))
            .await
            .unwrap();
        assert_eq!(host, "127.0.0.1");
        let y = web_yaml(dir.path());
        assert!(
            y.contains("bind: 127.0.0.1") && y.contains("host: 127.0.0.1"),
            "{y}"
        );
        assert!(y.contains(&format!("port: {port}")), "{y}");
        let wild = tempfile::TempDir::new().unwrap();
        let (_, host) = persist_bound(wild.path(), Some(free_port()), Some("0.0.0.0"))
            .await
            .unwrap();
        assert_eq!(host, "0.0.0.0");
        let y = web_yaml(wild.path());
        assert!(y.contains("bind: 0.0.0.0") && !y.contains("host:"), "{y}");
        // no --host: the record says nothing of the address
        let plain = tempfile::TempDir::new().unwrap();
        let (_, host) = persist_bound(plain.path(), Some(free_port()), None)
            .await
            .unwrap();
        assert_eq!(host, "127.0.0.1");
        let y = web_yaml(plain.path());
        assert!(!y.contains("bind:") && !y.contains("host:"), "{y}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn persist_web_without_a_port_keeps_the_recorded_one_else_picks_a_free_one() {
        let _web = WEB.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        let (picked, _) = persist_bound(dir.path(), None, None).await.unwrap();
        assert!(picked > 0);
        assert!(web_yaml(dir.path()).contains(&format!("port: {picked}")));
        let (kept, _) = persist_bound(dir.path(), None, None).await.unwrap();
        assert_eq!(kept, picked, "a recorded port is kept");
        let named = free_port();
        let (now, _) = persist_bound(dir.path(), Some(named), None).await.unwrap();
        assert_eq!(now, named, "a named port overwrites the recorded one");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn persist_web_refuses_a_bind_the_recorded_host_does_not_hold() {
        let _web = WEB.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        persist_bound(dir.path(), Some(free_port()), Some("0.0.0.0"))
            .await
            .unwrap();
        let err = persist_bound(dir.path(), None, Some("127.0.0.1"))
            .await
            .unwrap_err();
        assert!(
            err.contains("--host 127.0.0.1") && err.contains("still binds 0.0.0.0"),
            "{err}"
        );
        // the refusal released the lock
        persist_bound(dir.path(), None, Some("0.0.0.0"))
            .await
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn persist_web_is_idempotent() {
        let _web = WEB.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        let (p1, p2) = (free_port(), free_port());
        persist(dir.path(), p1).await.unwrap();
        persist(dir.path(), p2).await.unwrap();
        let k = start(dir.path()).await.unwrap();
        let id = |s: &str| fantastic_kernel::AgentId::from(s);
        let count = |h: &str| {
            k.agents
                .iter()
                .filter(|a| a.handler_module.as_deref() == Some(h))
                .count()
        };
        assert_eq!(count(fantastic_web::HANDLER_MODULE), 1);
        assert_eq!(count(page::HANDLER_MODULE), 1);
        assert_eq!(count(fantastic_web_ws::HANDLER_MODULE), 1);
        let port = k
            .agents
            .get(&id(page::WEB_ID))
            .unwrap()
            .meta
            .read()
            .unwrap()["port"]
            .clone();
        assert_eq!(port, json!(p2));
        k.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    /// #241: the kernel that writes the records serves nothing. While it is up, with
    /// the host just made (a fresh directory) and with the host loaded from its record
    /// (the next run), the port is free; a kernel with the host then serves the page.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_offline_kernel_that_writes_the_records_opens_no_port() {
        let _web = WEB.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        hydrate::seed(dir.path()).unwrap();
        for run in ["fresh", "recorded"] {
            let k = start_offline(dir.path()).await.unwrap();
            set_run_flags(&k, Some(port), None).await.unwrap();
            assert!(
                k.agents
                    .contains_key(&fantastic_kernel::AgentId::from(page::WEB_ID)),
                "{run}: the host is made"
            );
            if let Err(e) = std::net::TcpListener::bind(("127.0.0.1", port)) {
                panic!("{run}: the offline kernel listens on {port}: {e}");
            }
            k.shutdown().await;
            fantastic_sqlite_store::close_all(&k);
            bootstrap::shutdown(dir.path()).unwrap();
        }
        let k = start(dir.path()).await.unwrap();
        let resp = http_get(port, page::PAGE_PATH).await;
        assert_eq!(status_of(&resp), 200, "{resp}");
        k.shutdown().await;
        fantastic_sqlite_store::close_all(&k);
        bootstrap::shutdown(dir.path()).unwrap();
    }

    /// #241, `persist_web` itself: while the kernel that writes the records is up, with
    /// the host just made (a fresh directory) and loaded from its record (the next run,
    /// the recorded port kept or a new one named), the host is made and its port is free.
    #[tokio::test(flavor = "multi_thread")]
    async fn persist_web_writes_the_records_with_no_port_open() {
        let _web = WEB.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        for (run, port) in [
            ("fresh", Some(free_port())),
            ("recorded", None),
            ("renamed", Some(free_port())),
        ] {
            let path = dir.path().to_path_buf();
            let ((port, host), (made, bound)) = std::thread::spawn(move || {
                persist_web_then(&path, port, None, |k, (port, host)| {
                    let made = k
                        .agents
                        .contains_key(&fantastic_kernel::AgentId::from(page::WEB_ID));
                    let bound = std::net::TcpListener::bind((host.as_str(), *port)).err();
                    (made, bound.map(|e| e.to_string()))
                })
            })
            .join()
            .unwrap()
            .unwrap();
            assert!(made, "{run}: the host is made");
            assert_eq!(bound, None, "{run}: the kernel listens on {host}:{port}");
            assert!(
                web_yaml(dir.path()).contains(&format!("port: {port}")),
                "{run}: the record names the port"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn persist_web_busy_port_fails_and_releases_the_lock() {
        let _web = WEB.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        let busy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = busy.local_addr().unwrap().port();
        let err = persist(dir.path(), port)
            .await
            .expect_err("a busy port is an error");
        assert!(err.contains(&port.to_string()), "{err}");
        let mut h = TeamHandle::start(dir.path(), TeamOptions::default())
            .await
            .expect("the lock was released");
        h.stop().await.unwrap();
        // the refusal wrote no port: the seeded host is there without one
        let rec =
            std::fs::read_to_string(dir.path().join(".fantastic/agents/web/agent.yaml")).unwrap();
        assert!(!rec.contains("port:"), "{rec}");
        drop(busy);
    }

    /// fantastic refuses a web boot in which two children declare the same literal route
    /// (fantastic#446: "is already declared by"). The team's page and socket leg, mounted
    /// next to each other on the one host, must not clash.
    #[tokio::test(flavor = "multi_thread")]
    async fn team_web_routes_do_not_clash_at_boot() {
        let _web = WEB.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        persist(dir.path(), port).await.unwrap();
        // the boot of the host is the clash check: it must succeed and serve both routes
        let k = start(dir.path()).await.unwrap();
        let resp = http_get(port, page::PAGE_PATH).await;
        assert_eq!(status_of(&resp), 200, "{resp}");
        assert!(!resp.contains("is already declared by"), "{resp}");
        let (head, _c) = ws_open(port).await;
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");
        let web = k.agents.get(&fantastic_kernel::AgentId::from(page::WEB_ID));
        let status = web
            .map(|w| w.meta.read().unwrap().get("status").cloned())
            .unwrap_or(None);
        assert!(
            !status
                .map(|v| v.to_string().contains("already declared"))
                .unwrap_or(false),
            "the host recorded a route clash"
        );
        k.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }
}
