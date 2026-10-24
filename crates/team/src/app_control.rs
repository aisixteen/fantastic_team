//! In-process control of the headless team: start, pause, resume, stop. These
//! are the same kernel messages any client sends; there is no UI-only path.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use fantastic_kernel::bootstrap::{self, DEFAULT_ROOT_ID};
use fantastic_kernel::{Item, Kernel, Status};
use serde_json::{json, Value};

use crate::cadencer::HANDLER_MODULE;
use crate::start;

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
    /// A web UI port. Not wired into the handle yet; `None` is headless.
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
}

impl TeamHandle {
    /// Boot the team in `workdir` and start it. Fails if `opts.port` is set,
    /// because serving a port from a handle is not wired yet.
    pub async fn start(workdir: &Path, opts: TeamOptions) -> Result<TeamHandle, String> {
        if opts.port.is_some() {
            return Err("serving a port from a handle is not wired yet; start headless".into());
        }
        let kernel = start(workdir).await.map_err(|e| e.to_string())?;
        Ok(TeamHandle {
            dir: workdir.to_path_buf(),
            kernel: Some(kernel),
            state: State::Running,
            held: Vec::new(),
        })
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
            kernel.shutdown().await;
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
}
