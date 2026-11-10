//! The exec record a team gives its workers.
//!
//! A worker runs its commands through a fantastic `exec_bridge.tools` agent
//! built from [`worker_exec_record`]. Its HOME is a sandbox folder inside the
//! team with no credentials in it, and its environment is a fixed list of names;
//! no token, key or SSH agent is among them, so a push from a worker fails.
//!
//! The `allowed` pattern is not here: it is a value of the worker exec record in
//! `crates/team/blueprint.yaml`, any command line as in fantastic's own, and the user
//! can change it.
//!
//! This is NOT an OS sandbox. The worker runs as the user, so files the user
//! can read (`~/.ssh` keys, the login keychain) stay readable to a worker that
//! looks for them. Only a separate user or a container closes that.

use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// The fantastic bundle a worker's exec agent runs on.
pub const EXEC_HANDLER: &str = "exec_bridge.tools";
/// The worker's HOME, relative to the team folder.
pub const SANDBOX_HOME: &str = "sandbox/home";
/// Where a worker's commands run, relative to the team folder.
pub const WORK_DIR: &str = "work";

/// The ONLY names a worker's command inherits from the team's environment.
pub const INHERIT_ENV: &[&str] = &[
    "PATH", "LANG", "LC_ALL", "LC_CTYPE", "TERM", "TMPDIR", "TZ", "USER", "LOGNAME", "SHELL",
];

/// ssh for git with no config, no agent and no key files, never asking.
pub const NO_SSH: &str =
    "ssh -F /dev/null -o IdentityAgent=none -o IdentitiesOnly=yes -o IdentityFile=/dev/null -o BatchMode=yes";

/// Join a `/`-separated relative constant one component at a time, so the
/// result has the OS's separators (a single `join` of "a/b" keeps a mixed
/// `\\`-and-`/` path on Windows).
fn join_parts(base: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(base.to_path_buf(), |p, c| p.join(c))
}

/// The worker's HOME inside the team folder.
pub fn sandbox_home(team_dir: &Path) -> PathBuf {
    join_parts(team_dir, SANDBOX_HOME)
}

/// The folder a worker's commands run in, inside the team folder.
pub fn work_dir(team_dir: &Path) -> PathBuf {
    join_parts(team_dir, WORK_DIR)
}

/// The exec record for a worker. Pure: touches no disk.
pub fn worker_exec_record(team_dir: &Path) -> Value {
    let home = sandbox_home(team_dir);
    let s = |p: &Path| p.to_string_lossy().into_owned();
    let mut env = Map::new();
    env.insert("HOME".into(), json!(s(&home)));
    env.insert("XDG_CONFIG_HOME".into(), json!(s(&home.join(".config"))));
    env.insert("GIT_CONFIG_NOSYSTEM".into(), json!("1"));
    env.insert("GIT_TERMINAL_PROMPT".into(), json!("0"));
    env.insert("GCM_INTERACTIVE".into(), json!("never"));
    env.insert("GIT_SSH_COMMAND".into(), json!(NO_SSH));
    json!({
        "handler_module": EXEC_HANDLER,
        "ingress_rule": "allow_all_ingress",
        "cwd": s(&work_dir(team_dir)),
        "inherit_env": INHERIT_ENV,
        "env": env,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fantastic_kernel::Bundle;

    fn argv(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn refused(rec: &Value, argv: &[&str]) -> bool {
        fantastic_exec_bridge::command_permitted(
            rec.as_object().unwrap(),
            &fantastic_exec_bridge::shlex_join(&argv_owned(argv)),
        )
        .is_some()
    }

    fn argv_owned(v: &[&str]) -> Vec<String> {
        argv(v)
    }

    #[test]
    fn the_record_points_home_into_the_team() {
        let dir = tempfile::TempDir::new().unwrap();
        let rec = worker_exec_record(dir.path());
        let home = sandbox_home(dir.path());
        let env = &rec["env"];
        assert_eq!(env["HOME"], json!(home.to_string_lossy()));
        assert_eq!(
            env["XDG_CONFIG_HOME"],
            json!(home.join(".config").to_string_lossy())
        );
        assert_eq!(env["GIT_CONFIG_NOSYSTEM"], json!("1"));
        assert_eq!(env["GIT_TERMINAL_PROMPT"], json!("0"));
        assert_eq!(
            rec["cwd"],
            json!(work_dir(dir.path()).to_string_lossy().into_owned())
        );
        assert_eq!(rec["handler_module"], json!("exec_bridge.tools"));
    }

    #[test]
    fn sandbox_home_has_no_mixed_separators() {
        let dir = tempfile::TempDir::new().unwrap();
        let home = sandbox_home(dir.path());
        assert!(
            home.ends_with(Path::new("sandbox").join("home")),
            "{home:?}"
        );
        if cfg!(windows) {
            assert!(!home.to_string_lossy().contains('/'), "{home:?}");
        }
    }

    #[test]
    fn stock_record_passes_exec_validation_and_refuses_nothing() {
        // the record as the first run writes it: `allowed` is the blueprint entry's own value
        fn find(v: &Value) -> Option<Value> {
            match v {
                Value::Array(a) => a.iter().find_map(find),
                Value::Object(o) if o.get("id") == Some(&json!(crate::new_team::WORKER_EXEC)) => {
                    Some(v.clone())
                }
                Value::Object(o) => o.values().find_map(find),
                _ => None,
            }
        }
        let dir = tempfile::TempDir::new().unwrap();
        let doc: Value =
            serde_yaml::from_str(&crate::hydrate::blueprint_text(dir.path()).unwrap()).unwrap();
        let rec = find(&doc["agents"]).expect("the blueprint declares the worker exec");
        assert!(fantastic_exec_bridge::ExecBridgeBundle
            .validate_meta(rec.as_object().unwrap(), &Map::new())
            .is_none());
        for a in [
            &["gh", "--version"][..],
            &["git", "push"],
            &["glab", "mr", "list"],
            &["curl", "-s", "https://api.github.com/user"],
            &["sh", "-c", "gh auth token"],
            &["git", "status"],
        ] {
            assert!(
                !refused(&rec, a),
                "the general build refuses nothing: {a:?}"
            );
        }
    }
}
