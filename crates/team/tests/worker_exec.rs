use fantastic_kernel::bootstrap::{self, DEFAULT_ROOT_ID};
use fantastic_kernel::{Item, Kernel};
use fantastic_team::worker_exec::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const PLANTED_ENV: &[(&str, &str)] = &[
    ("GH_TOKEN", "planted-gh-token"),
    ("GITHUB_TOKEN", "planted-github-token"),
    ("SSH_AUTH_SOCK", "/tmp/planted-agent.sock"),
    ("GIT_ASKPASS", "/bin/planted-askpass"),
    ("SSH_ASKPASS", "/bin/planted-askpass"),
    ("FANTASTIC_TEAM_PLANTED_TOKEN", "planted-x-token"),
    ("FANTASTIC_TEAM_PLANTED_KEY", "planted-x-key"),
    ("ANTHROPIC_API_KEY", "planted-anthropic-key"),
];

static PLANT: OnceLock<TempDir> = OnceLock::new();

/// A credential-rich HOME and env for THIS test process, as a developer's shell has
/// them. Done once; every test calls it first, so nothing changes the env later.
fn plant() -> PathBuf {
    PLANT
        .get_or_init(|| {
            let t = TempDir::new().unwrap();
            let home = t.path().join("planted-home");
            std::fs::create_dir_all(home.join(".config/gh")).unwrap();
            std::fs::create_dir_all(home.join(".ssh")).unwrap();
            std::fs::write(
                home.join(".config/gh/hosts.yml"),
                "github.com:\n    oauth_token: planted-gh-hosts-token\n",
            )
            .unwrap();
            std::fs::write(home.join(".ssh/id_ed25519"), "planted-ssh-key\n").unwrap();
            std::fs::write(
                home.join(".git-credentials"),
                "https://planted-user:planted-password@github.com\n",
            )
            .unwrap();
            std::fs::write(
                home.join(".gitconfig"),
                "[credential]\n\thelper = \"!f() { echo username=planted-user; echo password=planted-password; }; f\"\n",
            )
            .unwrap();
            for (k, v) in PLANTED_ENV {
                std::env::set_var(k, v);
            }
            std::env::set_var("HOME", &home);
            t
        })
        .path()
        .join("planted-home")
}

async fn boot(team: &Path, id: &str, allowed: Option<Value>) -> Arc<Kernel> {
    prepare_sandbox(team).unwrap();
    let kernel = fantastic_team::start(team).await.unwrap();
    let mut rec = worker_exec_record(team);
    if let Some(a) = allowed {
        rec["allowed"] = a;
    }
    let mut create = rec.as_object().unwrap().clone();
    create.insert("type".into(), json!("create_agent"));
    create.insert("parent_id".into(), json!("kernel"));
    create.insert("id".into(), json!(id));
    let mut reply = kernel
        .send(vec![Item::new(DEFAULT_ROOT_ID, Value::Object(create))])
        .await;
    let result = reply.remove(0).result;
    println!("create_agent reply: {result}");
    assert!(
        result.get("error").is_none(),
        "create_agent failed: {result}"
    );
    kernel
}

async fn down(kernel: Arc<Kernel>, team: &Path) {
    kernel.shutdown().await;
    bootstrap::shutdown(team).unwrap();
}

async fn run(kernel: &Arc<Kernel>, id: &str, command: &str, args: &[&str]) -> Value {
    let mut reply = kernel
        .send(vec![Item::new(
            id,
            json!({"type": "run", "command": command, "args": args, "wait_s": 60}),
        )])
        .await;
    reply.remove(0).result
}

fn lines(reply: &Value) -> Vec<String> {
    reply["lines"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|l| l["line"].as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Test-local path normalization: forward slashes, no trailing slash, and lowercase
/// only on Windows. It does not map drive letters to MSYS prefixes (/c, /tmp).
fn norm(p: &str) -> String {
    let mut s = p.replace('\\', "/");
    while s.ends_with('/') {
        s.pop();
    }
    if cfg!(windows) {
        s = s.to_lowercase();
    }
    s
}

/// True when `value` names `<team>/sandbox/home<tail>`, in native or MSYS form. It
/// matches the team tempdir's unique final component plus the sandbox-relative tail,
/// so it is separator- and MSYS-insensitive, yet the planted home and other tempdirs
/// never match.
fn points_into_sandbox(value: &str, team: &Path, tail: &str) -> bool {
    let name = team.file_name().unwrap().to_string_lossy();
    let want = norm(&format!("{name}/sandbox/home{tail}"));
    let got = norm(value);
    got == want || got.ends_with(&format!("/{want}"))
}

/// A stand-in forge that answers EVERY request 401 and records whether any
/// request carried credentials.
async fn auth_wall() -> (u16, Arc<AtomicUsize>, Arc<AtomicBool>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let hits = Arc::new(AtomicUsize::new(0));
    let saw_auth = Arc::new(AtomicBool::new(false));
    let (h, a) = (hits.clone(), saw_auth.clone());
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else {
                return;
            };
            let (h, a) = (h.clone(), a.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match s.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                h.fetch_add(1, Ordering::SeqCst);
                if String::from_utf8_lossy(&buf)
                    .to_ascii_lowercase()
                    .contains("\nauthorization:")
                {
                    a.store(true, Ordering::SeqCst);
                }
                let _ = s.write_all(b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"team-test\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                let _ = s.shutdown().await;
            });
        }
    });
    (port, hits, saw_auth)
}

/// The test process's own git, with the planted HOME.
async fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    tokio::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_ASKPASS")
        .env_remove("SSH_ASKPASS")
        .output()
        .await
        .unwrap()
}

async fn repo_with_a_commit(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    assert!(git(dir, &["init", "-q"]).await.status.success());
    assert!(git(
        dir,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.test",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "x",
        ],
    )
    .await
    .status
    .success());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_worker_shell_sees_the_sandbox_home_and_no_credentials() {
    let planted = plant();
    let _ = planted;
    let team = TempDir::new().unwrap();
    let kernel = boot(team.path(), "worker_exec_env", None).await;
    let reply = run(&kernel, "worker_exec_env", "sh", &["-c", "echo $HOME; env"]).await;
    assert_eq!(reply["exit_code"], json!(0), "{reply}");
    let out = lines(&reply);
    // The shell may report paths in its own form (Git-for-Windows sh shows HOME
    // as /tmp/.tmpXXXX/...), so check that both point into this team's sandbox.
    assert!(
        out.first()
            .is_some_and(|h| points_into_sandbox(h, team.path(), "")),
        "{out:?}"
    );
    let env_home = out.iter().find_map(|l| l.strip_prefix("HOME="));
    assert!(
        env_home.is_some_and(|v| points_into_sandbox(v, team.path(), "")),
        "{out:?}"
    );
    let env_xdg = out.iter().find_map(|l| l.strip_prefix("XDG_CONFIG_HOME="));
    assert!(
        env_xdg.is_some_and(|v| points_into_sandbox(v, team.path(), "/.config")),
        "{out:?}"
    );
    assert!(!out.iter().any(|l| l.contains("planted-home")), "{out:?}");
    for l in &out {
        if let Some((name, _)) = l.split_once('=') {
            assert!(!forbidden_env_name(name), "forbidden name leaked: {name}");
            assert!(
                !PLANTED_ENV.iter().any(|(n, _)| *n == name),
                "planted name leaked: {name}"
            );
        }
        assert!(!l.contains("planted"), "planted value leaked: {l}");
    }
    down(kernel, team.path()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_push_from_a_worker_sends_no_credentials() {
    let planted = plant();
    let _ = planted;

    // CONTROL: the plant would leak if git used it.
    let control_dir = TempDir::new().unwrap();
    let (port, hits, saw) = auth_wall().await;
    let control = control_dir.path().join("control");
    repo_with_a_commit(&control).await;
    let url = format!("http://127.0.0.1:{port}/r.git");
    let out = git(&control, &["push", &url, "HEAD:refs/heads/x"]).await;
    assert!(!out.status.success());
    assert!(hits.load(Ordering::SeqCst) >= 1);
    assert!(
        saw.load(Ordering::SeqCst),
        "control: planted helper did not answer"
    );

    // WORKER: the same push through a record with only `allowed` widened.
    let team = TempDir::new().unwrap();
    let (port, hits, saw) = auth_wall().await;
    let url = format!("http://127.0.0.1:{port}/r.git");
    let kernel = boot(team.path(), "worker_exec_push", Some(json!([".*"]))).await;
    repo_with_a_commit(&work_dir(team.path())).await;
    let reply = run(
        &kernel,
        "worker_exec_push",
        "git",
        &["push", &url, "HEAD:refs/heads/x"],
    )
    .await;
    let code = reply["exit_code"].as_i64();
    assert!(code.map(|c| c != 0).unwrap_or(false), "{reply}");
    assert!(
        hits.load(Ordering::SeqCst) >= 1,
        "git did not reach the forge"
    );
    assert!(!saw.load(Ordering::SeqCst), "worker sent credentials");
    let joined = lines(&reply).join("\n");
    assert!(
        joined.contains("terminal prompts disabled")
            || joined.contains("could not read Username")
            || joined.contains("Authentication failed"),
        "{joined}"
    );
    down(kernel, team.path()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn forge_commands_are_refused_before_they_run() {
    let planted = plant();
    let _ = planted;
    let team = TempDir::new().unwrap();
    let kernel = boot(team.path(), "worker_exec_gate", None).await;
    let cases: [(&str, &[&str]); 3] = [
        ("gh", &["--version"]),
        ("git", &["push", "origin", "HEAD"]),
        ("sh", &["-c", "gh auth token"]),
    ];
    for (command, args) in cases {
        let reply = run(&kernel, "worker_exec_gate", command, args).await;
        assert_eq!(reply["reason"], json!("unauthorized"), "{reply}");
        assert!(
            reply["error"]
                .as_str()
                .unwrap_or("")
                .contains("command not allowed"),
            "{reply}"
        );
        assert!(reply.get("job_id").is_none(), "{reply}");
    }
    down(kernel, team.path()).await;
}

#[test]
fn sandbox_path_check_accepts_msys_and_windows_forms() {
    let team = Path::new("/var/folders/x/.tmpAbC");
    assert!(points_into_sandbox("/tmp/.tmpAbC/sandbox/home", team, ""));
    assert!(points_into_sandbox(
        "C:\\Users\\R~1\\AppData\\Local\\Temp\\.tmpAbC\\sandbox\\home\\.config",
        team,
        "/.config"
    ));
    assert!(points_into_sandbox(
        &sandbox_home(team).to_string_lossy(),
        team,
        ""
    ));
}

#[test]
fn sandbox_path_check_rejects_foreign_homes() {
    let team = Path::new("/var/folders/x/.tmpAbC");
    assert!(!points_into_sandbox("/home/dev", team, ""));
    assert!(!points_into_sandbox(
        "/var/folders/x/.tmpAbC/planted-home",
        team,
        ""
    ));
    assert!(!points_into_sandbox("/tmp/.tmpZzZ/sandbox/home", team, ""));
    assert!(!points_into_sandbox(
        "/tmp/xx.tmpAbC/sandbox/home",
        team,
        ""
    ));
    assert!(!points_into_sandbox(
        "/tmp/.tmpAbC/sandbox/home",
        team,
        "/.config"
    ));
    assert!(!points_into_sandbox(
        "/tmp/.tmpAbC/sandbox/home/.config",
        team,
        ""
    ));
}
