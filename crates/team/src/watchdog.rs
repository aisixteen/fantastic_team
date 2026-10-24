//! A backstop for shutdown (#119). fantastic's shell stops on SIGTERM/SIGINT by
//! draining every agent, which can wait on a model call that never answers.
//! That drain lives in the fantastic dependency; this repo cannot bound it, so
//! the binary runs a watchdog: after the first signal it gives the graceful
//! stop `grace`, then releases the lock, removes the run file and exits. A
//! second signal during the grace exits at once.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long the graceful stop may run after the first signal.
pub const GRACE: Duration = Duration::from_secs(4);

/// The process exit code for a death by `sig` (`128 + sig`).
pub fn exit_code(sig: i32) -> i32 {
    128 + sig
}

/// Signal numbers the watchdog reports.
const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;

/// Release `dir`'s kernel lock and remove its run file. Best effort: a missing
/// file or an already released lock is not an error.
pub fn force_exit_cleanup(dir: &Path) {
    let _ = fantastic_kernel::bootstrap::shutdown(dir);
    let _ = std::fs::remove_file(dir.join(".fantastic/cli/canvas.yaml"));
}

/// Start the watchdog on its own thread with its own runtime, so it still runs
/// while the shell's runtime is stuck in the drain.
pub fn spawn(dir: PathBuf, grace: Duration) {
    let _ = std::thread::Builder::new()
        .name("shutdown-watchdog".into())
        .spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            rt.block_on(watch(dir, grace));
        });
}

#[cfg(unix)]
async fn next_signal(
    term: &mut tokio::signal::unix::Signal,
    int: &mut tokio::signal::unix::Signal,
) -> i32 {
    tokio::select! {
        _ = term.recv() => SIGTERM,
        _ = int.recv() => SIGINT,
    }
}

#[cfg(unix)]
async fn watch(dir: PathBuf, grace: Duration) {
    use tokio::signal::unix::{signal, SignalKind};
    let (Ok(mut term), Ok(mut int)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
    ) else {
        return;
    };
    let first = next_signal(&mut term, &mut int).await;
    let code = tokio::select! {
        _ = tokio::time::sleep(grace) => exit_code(first),
        second = next_signal(&mut term, &mut int) => exit_code(second),
    };
    eprintln!(
        "fantastic_team: shutdown did not finish in {}s, exiting",
        grace.as_secs()
    );
    force_exit_cleanup(&dir);
    std::process::exit(code);
}

#[cfg(not(unix))]
async fn watch(dir: PathBuf, grace: Duration) {
    if tokio::signal::ctrl_c().await.is_err() {
        return;
    }
    tokio::select! {
        _ = tokio::time::sleep(grace) => {}
        _ = tokio::signal::ctrl_c() => {}
    }
    eprintln!(
        "fantastic_team: shutdown did not finish in {}s, exiting",
        grace.as_secs()
    );
    force_exit_cleanup(&dir);
    std::process::exit(exit_code(SIGINT));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_maps_signals() {
        assert_eq!(exit_code(SIGTERM), 143);
        assert_eq!(exit_code(SIGINT), 130);
    }

    #[test]
    fn force_exit_cleanup_removes_lock_and_run_file() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join(".fantastic");
        std::fs::create_dir_all(state.join("cli")).unwrap();
        fantastic_kernel::lock::acquire(dir.path()).unwrap();
        assert!(state.join("lock.yaml").exists());
        std::fs::write(state.join("cli/canvas.yaml"), "pid: 1\n").unwrap();
        force_exit_cleanup(dir.path());
        assert!(!state.join("cli/canvas.yaml").exists());
        assert!(!state.join("lock.yaml").exists());
        // Nothing left to remove: still fine.
        force_exit_cleanup(dir.path());
    }
}
