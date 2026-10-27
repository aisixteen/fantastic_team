//! In-process control of the headless team: start, pause, resume, stop. These
//! are the same kernel messages any client sends; there is no UI-only path.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use fantastic_kernel::bootstrap::{self, DEFAULT_ROOT_ID};
use fantastic_kernel::{Item, Kernel, Status};
use serde_json::{json, Value};

use crate::cadencer::HANDLER_MODULE;
use crate::{page, start, tree};

/// The dispatcher verbs the web page's socket leg may call.
const PAGE_VERBS: [&str; 4] = ["sidebar", "tree", "open", "control"];

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
    /// A web UI port on 127.0.0.1. `Some(p)`: the handle serves the dispatcher page
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
    /// the dispatcher page on that port; when that fails the kernel is shut down
    /// and the directory lock released before the error is returned.
    pub async fn start(workdir: &Path, opts: TeamOptions) -> Result<TeamHandle, String> {
        let kernel = start(workdir).await.map_err(|e| e.to_string())?;
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

/// Serve the dispatcher page on `127.0.0.1:port`: a `web` host under the root, the
/// page agent and the socket leg under it ([`prepare_web`]), then boot the host (it
/// mounts the children, then listens). A taken port makes the boot fail. Safe to
/// repeat: what exists is kept, and the host's port is set.
async fn serve_web(kernel: &Arc<Kernel>, port: u16) -> Result<(), String> {
    prepare_web(kernel, port).await?;
    send_one(kernel, page::WEB_ID, json!({"type": "boot"}))
        .await
        .map(|_| ())
        .map_err(|e| format!("serving the web UI on port {port} failed: {e}"))
}

/// Everything [`serve_web`] does before the boot: the port pre-check, the `web`
/// host, the page and the socket leg, their edges, the leg's grants on the
/// dispatcher and the workers, and the records flushed to disk.
async fn prepare_web(kernel: &Arc<Kernel>, port: u16) -> Result<(), String> {
    let live = |id: &str| {
        kernel
            .agents
            .contains_key(&fantastic_kernel::AgentId::from(id))
    };
    // The host binds in a task of its own, so a taken port would only be logged.
    if !live(page::WEB_ID) {
        std::net::TcpListener::bind(("127.0.0.1", port))
            .map_err(|e| format!("serving the web UI on port {port} failed: {e}"))?;
    }
    if live(page::WEB_ID) {
        let upd = json!({"type": "update_agent", "id": page::WEB_ID, "port": port});
        send_one(kernel, DEFAULT_ROOT_ID, upd).await?;
    } else {
        let rec = json!({"type": "create_agent", "id": page::WEB_ID, "parent_id": DEFAULT_ROOT_ID,
            "handler_module": fantastic_web::HANDLER_MODULE, "port": port});
        send_one(kernel, DEFAULT_ROOT_ID, rec).await?;
    }
    if !live(page::PAGE_ID) {
        let rec = json!({"type": "create_agent", "id": page::PAGE_ID, "parent_id": page::WEB_ID,
            "handler_module": page::HANDLER_MODULE,
            "verbs": {"read_stream": page::READ_STREAM_DOC}});
        send_one(kernel, DEFAULT_ROOT_ID, rec).await?;
    }
    if !live(page::WS_ID) {
        // The leg is the page's one principal: it may call the dispatcher's page verbs
        // and the chat verbs of the workers, nothing else.
        let rec = json!({"type": "create_agent", "id": page::WS_ID, "parent_id": page::WEB_ID,
            "handler_module": fantastic_web_ws::HANDLER_MODULE,
            "ingress_rule": "allow_all_ingress"});
        send_one(kernel, DEFAULT_ROOT_ID, rec).await?;
    }
    // `web` mounts the leg by asking it for its routes, and serves the page by
    // reading `read_stream` as itself: both ends of both edges.
    let web = fantastic_kernel::AgentId::from(page::WEB_ID);
    for (served, verb) in [(page::WS_ID, "get_routes"), (page::PAGE_ID, "read_stream")] {
        let verbs = json!([verb]);
        kernel.edit_callable_by(&fantastic_kernel::AgentId::from(served), |cb| {
            cb.insert(page::WEB_ID.into(), verbs.clone());
            true
        })?;
        kernel.edit_can_call(&web, |cc| {
            cc.insert(served.into(), verbs.clone());
            true
        })?;
    }
    let leg = fantastic_kernel::AgentId::from(page::WS_ID);
    let dispatcher = fantastic_kernel::AgentId::from(tree::DISPATCHER_ID);
    if live(tree::DISPATCHER_ID) {
        let verbs = json!(PAGE_VERBS);
        kernel.edit_can_call(&leg, |cc| {
            cc.insert(tree::DISPATCHER_ID.into(), verbs.clone());
            true
        })?;
        kernel.edit_callable_by(&dispatcher, |cb| {
            cb.insert(page::WS_ID.into(), verbs.clone());
            true
        })?;
        let workers: Vec<String> = kernel
            .agents
            .get(&dispatcher)
            .and_then(|a| a.meta.read().ok().map(|m| m.get("groups").cloned()))
            .flatten()
            .and_then(|g| g.as_object().cloned())
            .map(|g| {
                g.values()
                    .flat_map(|m| m.as_array().cloned().unwrap_or_default())
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        tree::grant_chat_to(kernel, &workers, page::WS_ID);
    }
    kernel.flush_records().await;
    Ok(())
}

/// Write the web UI's records (the same ones [`serve_web`] makes, with the same
/// grants) into `dir` without serving them, for a host that boots the directory
/// itself: the binary's `--port` hands the records to fantastic's shell, which
/// hydrates and boots them. Runs an offline kernel on its own runtime and
/// releases the directory lock before it returns, on failure too. Safe to repeat.
pub fn persist_web(dir: &Path, port: u16) -> Result<(), String> {
    let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    rt.block_on(async {
        let kernel = start(dir).await.map_err(|e| e.to_string())?;
        let res = prepare_web(&kernel, port).await;
        if res.is_err() {
            drop_web(&kernel).await;
        }
        kernel.shutdown().await;
        fantastic_sqlite_store::close_all(&kernel);
        let released = bootstrap::shutdown(dir).map_err(|e| e.to_string());
        match (res, released) {
            (Ok(()), r) => r,
            (Err(e), Ok(())) => Err(format!("serving the web UI on port {port} failed: {e}")),
            (Err(e), Err(r)) => Err(format!(
                "serving the web UI on port {port} failed: {e}; releasing the lock failed: {r}"
            )),
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
    static WEB: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
        assert!(
            resp.contains("function renderSidebar("),
            "the dispatcher page is served"
        );
        assert!(
            resp.contains("window.fantastic = {"),
            "with the socket shim"
        );
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
        std::thread::spawn(move || persist_web(&dir, port))
            .join()
            .unwrap()
    }

    async fn make_dispatcher(k: &Arc<Kernel>, groups: Value) {
        let mk = json!({"type": "create_agent", "id": "dispatcher", "parent_id": DEFAULT_ROOT_ID,
            "handler_module": crate::dispatcher::HANDLER_MODULE, "groups": groups});
        send_one(k, DEFAULT_ROOT_ID, mk).await.unwrap();
    }

    /// The page's socket leg reaches the dispatcher: a raw websocket handshake and one
    /// `call` frame for `sidebar`, answered by a `reply` frame with the same id.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_page_socket_reaches_the_dispatcher() {
        let _web = WEB.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        // The dispatcher exists before the host is served, as after a first start.
        let k = start(dir.path()).await.unwrap();
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
        assert!(
            resp.contains("function renderSidebar("),
            "the page is served"
        );
        assert!(
            resp.contains("window.fantastic = {"),
            "with the socket shim"
        );
        assert_eq!(
            status_of(&http_get(port, "/team_page/file/other.txt").await),
            404
        );
        k.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn persist_web_socket_reaches_the_dispatcher() {
        let _web = WEB.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        let k = start(dir.path()).await.unwrap();
        make_dispatcher(&k, json!({})).await;
        k.flush_records().await;
        k.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
        persist(dir.path(), port).await.unwrap();
        let k = start(dir.path()).await.unwrap();
        let (head, _c) = ws_open(port).await;
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");
        let reply = ws_call(port, "dispatcher", json!({"type": "sidebar"})).await;
        assert_eq!(reply["type"], "reply", "{reply}");
        assert_eq!(reply["id"], "p1");
        assert_eq!(reply["data"]["groups"], json!([]), "{reply}");
        k.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn persist_web_grants_chat_to_the_workers() {
        let _web = WEB.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        let k = start(dir.path()).await.unwrap();
        let mk = json!({"type": "create_agent", "id": "w1", "parent_id": DEFAULT_ROOT_ID,
            "handler_module": "ram_store.tools"});
        send_one(&k, DEFAULT_ROOT_ID, mk).await.unwrap();
        make_dispatcher(&k, json!({"workers": ["w1"]})).await;
        k.flush_records().await;
        k.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
        persist(dir.path(), free_port()).await.unwrap();
        let k = start(dir.path()).await.unwrap();
        let id = |s: &str| fantastic_kernel::AgentId::from(s);
        let meta =
            |a: &str, key: &str| k.agents.get(&id(a)).unwrap().meta.read().unwrap()[key].clone();
        let cb = meta("w1", "callable_by");
        let grant = cb.get(page::WS_ID).expect("team_ws may call w1");
        for verb in ["history", "send", "stop", "status", "watch"] {
            assert!(grant.as_array().unwrap().contains(&json!(verb)), "{verb}");
        }
        let cc = meta(page::WS_ID, "can_call");
        assert_eq!(cc.get("dispatcher"), Some(&json!(PAGE_VERBS)), "{cc:?}");
        assert_eq!(
            cc.get("w1"),
            Some(&json!(crate::tree::CHAT_VERBS)),
            "{cc:?}"
        );
        k.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
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
        assert!(!h
            .kernel
            .as_ref()
            .unwrap()
            .agents
            .contains_key(&fantastic_kernel::AgentId::from(page::WEB_ID)));
        h.stop().await.unwrap();
        drop(busy);
    }
}
