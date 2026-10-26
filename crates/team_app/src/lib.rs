//! App shell for the headless team binary's web UI (#37, part a).
//! No logic here: resolve a target, start or attach, show its URL.

use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};

/// The headless binary name (`[[bin]] name` in crates/team/Cargo.toml).
pub const HEADLESS_BINARY: &str = "fantastic_team";

/// The loopback port the packaged window loads (`app/tauri.conf.json`).
pub const DEFAULT_PORT: u16 = 8765;

/// The ssh program a tunnel runs unless `--ssh` names another.
pub const DEFAULT_SSH: &str = "ssh";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Start {
        binary: String,
        port: u16,
    },
    Attach {
        url: String,
    },
    /// A web UI on another machine, reached over the user's own `ssh -L`. The
    /// keys stay in the user's ssh-agent; the app never sees them.
    Tunnel {
        ssh: String,
        host: String,
        remote_port: u16,
        local_port: u16,
    },
    /// The GitHub App setup flow of the headless binary (`setup github`).
    Setup {
        binary: String,
        name: String,
        org: Option<String>,
    },
}

/// A host for `--tunnel`: letters, digits and `. _ @ -`, not starting with `-`
/// (so it is never read by ssh as an option).
fn valid_ssh_host(h: &str) -> bool {
    !h.is_empty()
        && !h.starts_with('-')
        && h.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | '-'))
}

/// A GitHub app or org name for `--setup-github`: `[A-Za-z0-9-]{1,39}`.
fn valid_slug(s: &str) -> bool {
    (1..=39).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Parse the value of `--tunnel`: `HOST` or `HOST:REMOTE_PORT`. The error never
/// repeats the value.
fn parse_tunnel(v: &str) -> Result<(String, u16), String> {
    let bad = || "bad --tunnel: expected HOST or HOST:PORT (letters, digits, . _ @ -)".to_string();
    let (host, port) = match v.rsplit_once(':') {
        Some((h, p)) => {
            let n: u16 = p.parse().map_err(|_| bad())?;
            if n == 0 {
                return Err(bad());
            }
            (h, n)
        }
        None => (v, DEFAULT_PORT),
    };
    if !valid_ssh_host(host) {
        return Err(bad());
    }
    Ok((host.to_string(), port))
}

/// The ssh command line of a tunnel: forward a loopback port to the remote
/// machine's loopback port and nothing else. No key, no other option.
pub fn tunnel_args(host: &str, remote_port: u16, local_port: u16) -> Vec<String> {
    vec![
        "-N".to_string(),
        "-o".to_string(),
        "ExitOnForwardFailure=yes".to_string(),
        "-L".to_string(),
        format!("127.0.0.1:{local_port}:127.0.0.1:{remote_port}"),
        host.to_string(),
    ]
}

/// The headless binary's setup command line: `setup github --name N [--org O]`.
/// Names are checked when the flags are parsed; no secret ever goes here.
pub fn setup_args(name: &str, org: Option<&str>) -> Vec<String> {
    let mut v: Vec<String> = ["setup", "github", "--name", name]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if let Some(o) = org {
        v.push("--org".to_string());
        v.push(o.to_string());
    }
    v
}

/// The setup page's URL in a line of the child's output: the first word that is
/// `http://127.0.0.1:<port>/...`. Anything else gives None.
pub fn setup_url(line: &str) -> Option<String> {
    line.split_whitespace()
        .find(|w| {
            w.strip_prefix("http://127.0.0.1:")
                .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()))
                && validate_loopback(w).is_ok()
        })
        .map(str::to_string)
}

/// A port the OS says is free on loopback right now.
pub fn free_port() -> std::io::Result<u16> {
    let l = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(l.local_addr()?.port())
}

fn next_value<'a, I: Iterator<Item = &'a String>>(
    it: &mut I,
    flag: &str,
) -> Result<String, String> {
    it.next()
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}

/// Parse `--attach <url>` / `--port <n>` / `--binary <path>` / `--tunnel
/// HOST[:PORT]` (with `--ssh <path>`) / `--setup-github NAME` (with `--org O`).
/// With no flags: start the headless binary on `DEFAULT_PORT`.
pub fn target_from_args(args: &[String]) -> Result<Target, String> {
    let mut attach = None;
    let mut port: Option<u16> = None;
    let mut binary = None;
    let mut tunnel = None;
    let mut ssh = None;
    let mut setup = None;
    let mut org = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--attach" => attach = Some(next_value(&mut it, arg)?),
            "--port" => {
                let v = next_value(&mut it, arg)?;
                let p: u16 = v.parse().map_err(|_| format!("bad --port: {v}"))?;
                if p == 0 {
                    return Err("--port must be 1-65535".into());
                }
                port = Some(p);
            }
            "--binary" => binary = Some(next_value(&mut it, arg)?),
            "--tunnel" => tunnel = Some(parse_tunnel(&next_value(&mut it, arg)?)?),
            "--ssh" => ssh = Some(next_value(&mut it, arg)?),
            "--setup-github" => {
                let v = next_value(&mut it, arg)?;
                if !valid_slug(&v) {
                    return Err("bad --setup-github: use 1-39 letters, digits or -".into());
                }
                setup = Some(v);
            }
            "--org" => {
                let v = next_value(&mut it, arg)?;
                if !valid_slug(&v) {
                    return Err("bad --org: use 1-39 letters, digits or -".into());
                }
                org = Some(v);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if org.is_some() && setup.is_none() {
        return Err("--org needs --setup-github".into());
    }
    if ssh.is_some() && tunnel.is_none() {
        return Err("--ssh needs --tunnel".into());
    }
    if let Some(name) = setup {
        if attach.is_some() || port.is_some() || tunnel.is_some() {
            return Err(
                "--setup-github cannot be combined with --attach, --port or --tunnel".into(),
            );
        }
        return Ok(Target::Setup {
            binary: binary.unwrap_or_else(|| HEADLESS_BINARY.to_string()),
            name,
            org,
        });
    }
    if let Some((host, remote_port)) = tunnel {
        if attach.is_some() || port.is_some() || binary.is_some() {
            return Err("--tunnel cannot be combined with --attach, --port or --binary".into());
        }
        let local_port = free_port().map_err(|e| format!("no free local port: {e}"))?;
        return Ok(Target::Tunnel {
            ssh: ssh.unwrap_or_else(|| DEFAULT_SSH.to_string()),
            host,
            remote_port,
            local_port,
        });
    }
    if let Some(url) = attach {
        if port.is_some() || binary.is_some() {
            return Err("--attach cannot be combined with --port or --binary".into());
        }
        validate_loopback(&url)?;
        return Ok(Target::Attach { url });
    }
    let port = match port {
        Some(p) => p,
        None => DEFAULT_PORT,
    };
    Ok(Target::Start {
        binary: binary.unwrap_or_else(|| HEADLESS_BINARY.to_string()),
        port,
    })
}

/// Pull `--dir <path>` out of the args; the rest goes to `target_from_args`.
pub fn take_dir(args: &[String]) -> Result<(Option<PathBuf>, Vec<String>), String> {
    let mut dir = None;
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--dir" {
            dir = Some(PathBuf::from(next_value(&mut it, arg)?));
        } else {
            rest.push(arg.clone());
        }
    }
    Ok((dir, rest))
}

/// Split `http://authority/path` into (authority, path).
fn split_http(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("http://")?;
    Some(match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    })
}

/// Attach URLs must be plain http on loopback (tunnel/relay = loopback), with no
/// userinfo, query or fragment. An error never repeats the URL.
pub fn validate_loopback(url: &str) -> Result<(), String> {
    let (auth, _) =
        split_http(url).ok_or_else(|| "attach URL must start with http://".to_string())?;
    if url.contains('?') || url.contains('#') {
        return Err("attach URL must have no query or fragment".into());
    }
    if auth.contains('@') {
        return Err("attach URL must have no user info".into());
    }
    let host = if let Some(rest) = auth.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        auth.split(':').next().unwrap_or("")
    };
    if host == "127.0.0.1" || host == "localhost" || host == "::1" {
        Ok(())
    } else {
        Err(format!(
            "attach URL host must be 127.0.0.1 or localhost, got {host}"
        ))
    }
}

/// The headless command line: `--port N` and nothing else.
pub fn start_args(port: u16) -> Vec<String> {
    vec!["--port".to_string(), port.to_string()]
}

/// The web UI URL the window should show.
pub fn web_url(target: &Target) -> String {
    match target {
        Target::Start { port, .. } => format!("http://127.0.0.1:{port}/"),
        Target::Attach { url } => url.clone(),
        Target::Tunnel { local_port, .. } => format!("http://127.0.0.1:{local_port}/"),
        // the setup flow prints its own page URL; there is none to wait for here
        Target::Setup { .. } => String::new(),
    }
}

/// The page the headless binary's `--port` serves since #181 (the team crate's
/// `PAGE_PATH`). The web host answers 404 at `/`, so this is what the launcher
/// waits for; the window URL is still [`web_url`].
pub const READY_PATH: &str = "/team_page/file/index.html";

/// The URL the launcher polls for a 200. `Attach` is the URL the user named.
pub fn ready_url(target: &Target) -> String {
    match target {
        Target::Start { port, .. } => format!("http://127.0.0.1:{port}{READY_PATH}"),
        Target::Attach { url } => url.clone(),
        Target::Tunnel { local_port, .. } => format!("http://127.0.0.1:{local_port}{READY_PATH}"),
        Target::Setup { .. } => String::new(),
    }
}

async fn http_status(addr: &str, host: &str, path: &str) -> std::io::Result<u16> {
    let mut s = TcpStream::connect(addr).await?;
    let req = format!("GET {path} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    s.write_all(req.as_bytes()).await?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await?;
    String::from_utf8_lossy(&buf)
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| std::io::Error::other("bad HTTP response"))
}

/// A connect address for an authority: `host` gets port 80, `host:port` is kept.
/// A bracketed IPv6 host counts as having a port only after the closing `]`.
pub fn authority_addr(auth: &str) -> String {
    let has_port = match auth.rfind(']') {
        Some(i) => auth[i + 1..].starts_with(':'),
        None => auth.contains(':'),
    };
    if has_port {
        auth.to_string()
    } else {
        format!("{auth}:80")
    }
}

/// Poll `url` until it answers 200 or `timeout` passes.
pub async fn wait_ready(url: &str, timeout: Duration) -> Result<(), String> {
    let (auth, path) = split_http(url).ok_or_else(|| "not an http:// URL".to_string())?;
    let addr = authority_addr(auth);
    let deadline = Instant::now() + timeout;
    loop {
        let attempt = tokio::time::timeout(Duration::from_secs(2), http_status(&addr, auth, path));
        if let Ok(Ok(200)) = attempt.await {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("no 200 from {url} within {timeout:?}"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A running (or attached) web UI. For `Start`, the child is killed on drop.
pub struct Launched {
    child: Option<Child>,
    pub url: String,
}

/// Start the headless binary (or attach to a URL) and wait until its web UI
/// answers 200. If the child exits first, the error carries its exit status.
pub async fn launch(
    target: &Target,
    dir: Option<&Path>,
    ready_timeout: Duration,
) -> Result<Launched, String> {
    let url = web_url(target);
    let ready = ready_url(target);
    let (program, args, with_dir) = match target {
        Target::Start { binary, port } => (binary, start_args(*port), true),
        Target::Tunnel {
            ssh,
            host,
            remote_port,
            local_port,
        } => (ssh, tunnel_args(host, *remote_port, *local_port), false),
        Target::Attach { .. } => {
            wait_ready(&ready, ready_timeout).await?;
            return Ok(Launched { child: None, url });
        }
        Target::Setup { .. } => return Err("setup is started with start_setup".to_string()),
    };
    let binary = program;
    let mut cmd = Command::new(binary);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    if let (true, Some(d)) = (with_dir, dir) {
        cmd.arg("--dir").arg(d);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to start {binary}: {e}"))?;
    enum Outcome {
        Exited(Result<ExitStatus, std::io::Error>),
        Ready(Result<(), String>),
    }
    let outcome = tokio::select! {
        status = child.wait() => Outcome::Exited(status),
        r = wait_ready(&ready, ready_timeout) => Outcome::Ready(r),
    };
    match outcome {
        Outcome::Exited(status) => Err(match status {
            Ok(s) => format!("{binary} exited before serving {ready}: {s}"),
            Err(e) => format!("{binary} exited before serving {ready}: {e}"),
        }),
        Outcome::Ready(Ok(())) => Ok(Launched {
            child: Some(child),
            url,
        }),
        Outcome::Ready(Err(e)) => {
            let _ = child.kill().await;
            Err(e)
        }
    }
}

/// Run `<binary> setup github --name N [--org O]`: forward the child's stdout and
/// wait for the page URL it prints (see [`setup_url`]). The child stays up, with
/// its remaining output still forwarded, until the flow ends or `stop` kills it.
/// Nothing but [`setup_args`] reaches the child.
pub async fn start_setup(
    binary: &str,
    name: &str,
    org: Option<&str>,
    timeout: Duration,
) -> Result<Launched, String> {
    let mut child = Command::new(binary)
        .args(setup_args(name, org))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("failed to start {binary}: {e}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or("no stdout from the setup child")?;
    let mut lines = BufReader::new(stdout).lines();
    let found = tokio::time::timeout(timeout, async {
        while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
            println!("{line}");
            if let Some(url) = setup_url(&line) {
                return Ok(url);
            }
        }
        Err("the setup ended without a page URL".to_string())
    })
    .await;
    match found {
        Ok(Ok(url)) => {
            tokio::spawn(async move {
                while let Ok(Some(line)) = lines.next_line().await {
                    println!("{line}");
                }
            });
            Ok(Launched {
                child: Some(child),
                url,
            })
        }
        Ok(Err(e)) => {
            let _ = child.kill().await;
            Err(e)
        }
        Err(_) => {
            let _ = child.kill().await;
            Err(format!("no setup page URL within {timeout:?}"))
        }
    }
}

impl Launched {
    /// Wait for the child to exit. Never returns for an attached web UI.
    pub async fn exited(&mut self) -> std::io::Result<ExitStatus> {
        match self.child.as_mut() {
            Some(c) => c.wait().await,
            None => std::future::pending().await,
        }
    }

    /// Kill the child if it is still running. Safe to call twice; a no-op
    /// for an attached web UI.
    pub async fn stop(&mut self) -> Result<(), String> {
        let Some(child) = self.child.as_mut() else {
            return Ok(());
        };
        if child.try_wait().map_err(|e| e.to_string())?.is_none() {
            child.kill().await.map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

pub mod window;

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn default_is_start_on_the_packaged_port() {
        match target_from_args(&[]).unwrap() {
            Target::Start { binary, port } => {
                assert_eq!(binary, HEADLESS_BINARY);
                assert_eq!(port, DEFAULT_PORT);
            }
            other => panic!("expected Start, got {other:?}"),
        }
    }

    #[test]
    fn attach_parses_a_loopback_url() {
        let t = target_from_args(&args(&["--attach", "http://127.0.0.1:8080/"])).unwrap();
        assert_eq!(
            t,
            Target::Attach {
                url: "http://127.0.0.1:8080/".into()
            }
        );
        assert!(target_from_args(&args(&["--attach", "http://localhost:9/"])).is_ok());
        assert!(target_from_args(&args(&["--attach", "http://[::1]:8080/"])).is_ok());
    }

    #[test]
    fn attach_refuses_a_non_loopback_url() {
        assert!(target_from_args(&args(&["--attach", "http://example.com/"])).is_err());
        assert!(target_from_args(&args(&["--attach", "https://127.0.0.1/"])).is_err());
    }

    #[test]
    fn start_args_are_only_the_port() {
        assert_eq!(start_args(4321), args(&["--port", "4321"]));
    }

    #[tokio::test]
    async fn wait_ready_sees_a_local_server() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let _ = s.read(&mut buf).await;
                    let _ = s
                        .write_all(b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\nok")
                        .await;
                    let _ = s.shutdown().await;
                });
            }
        });
        wait_ready(&format!("http://127.0.0.1:{port}/"), Duration::from_secs(5))
            .await
            .unwrap();
    }

    #[test]
    fn default_port_matches_packaged_window() {
        let conf = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/tauri.conf.json"),
        )
        .expect("read app/tauri.conf.json");
        let rest = conf
            .split("http://127.0.0.1:")
            .nth(1)
            .expect("conf names a loopback url");
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        let packaged: u16 = digits.parse().expect("packaged port");
        assert_eq!(DEFAULT_PORT, packaged);
        match target_from_args(&[]).unwrap() {
            Target::Start { port, .. } => assert_eq!(port, DEFAULT_PORT),
            other => panic!("expected Start, got {other:?}"),
        }
    }

    #[test]
    fn take_dir_pulls_out_the_dir_flag() {
        let (dir, rest) = take_dir(&args(&["--dir", "/x", "--port", "9"])).unwrap();
        assert_eq!(dir, Some(std::path::PathBuf::from("/x")));
        assert_eq!(rest, args(&["--port", "9"]));
        assert!(take_dir(&args(&["--dir"])).is_err());
    }

    #[test]
    fn authority_keeps_an_explicit_port_and_adds_80_otherwise() {
        assert_eq!(authority_addr("[::1]"), "[::1]:80");
        assert_eq!(authority_addr("[::1]:8080"), "[::1]:8080");
        assert_eq!(authority_addr("127.0.0.1"), "127.0.0.1:80");
        assert_eq!(authority_addr("127.0.0.1:8080"), "127.0.0.1:8080");
    }

    /// A loopback listener and its port. The socket stays bound for as long
    /// as the caller holds it, so no other test can be handed the same port.
    async fn bind_free() -> (TcpListener, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        (listener, port)
    }

    /// Serve `200 ok` to every connection until the test ends.
    fn serve_200(listener: TcpListener) {
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let _ = s.read(&mut buf).await;
                    let _ = s
                        .write_all(b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\nok")
                        .await;
                    let _ = s.shutdown().await;
                });
            }
        });
    }

    /// Answer 200 only for `GET ok_path`, 404 for anything else (the real binary).
    fn serve_by_path(listener: TcpListener, ok_path: &'static str) {
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let n = s.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let ok = req.split_whitespace().nth(1) == Some(ok_path);
                    let resp: &[u8] = if ok {
                        b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\nok"
                    } else {
                        b"HTTP/1.0 404 Not Found\r\nContent-Length: 0\r\n\r\n"
                    };
                    let _ = s.write_all(resp).await;
                    let _ = s.shutdown().await;
                });
            }
        });
    }

    /// Serialises "write a script, then exec it" across tests. A fork in a
    /// parallel test can inherit our still-open write fd and make the exec fail
    /// with ETXTBSY ("Text file busy") until that child execs.
    #[cfg(unix)]
    static EXEC_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[cfg(unix)]
    fn fake_binary(dir: &std::path::Path, name: &str, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn launch_starts_child_and_returns_url_when_ready() {
        let _exec = EXEC_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_binary(dir.path(), "fake", "exec sleep 30");
        let (listener, port) = bind_free().await;
        serve_by_path(listener, READY_PATH);
        let target = Target::Start { binary: bin, port };
        let mut l = launch(&target, None, Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(l.url, web_url(&target));
        assert!(l.child.as_ref().unwrap().id().is_some());
        l.stop().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn launch_reports_child_that_exits_early() {
        let _exec = EXEC_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_binary(dir.path(), "fake", "exit 3");
        // Held, never served: no other test can take this port and answer 200.
        let (_held, port) = bind_free().await;
        let target = Target::Start { binary: bin, port };
        let started = Instant::now();
        let err = match launch(&target, None, Duration::from_secs(20)).await {
            Ok(_) => panic!("launch should fail when the child exits"),
            Err(e) => e,
        };
        assert!(err.contains("exit"), "error names the exit: {err}");
        assert!(err.contains('3'), "error carries the status: {err}");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stop_kills_the_child_and_is_idempotent() {
        let _exec = EXEC_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_binary(dir.path(), "fake", "exec sleep 30");
        let (listener, port) = bind_free().await;
        serve_by_path(listener, READY_PATH);
        let target = Target::Start { binary: bin, port };
        let mut l = launch(&target, None, Duration::from_secs(10))
            .await
            .unwrap();
        l.stop().await.unwrap();
        let child = l.child.as_mut().unwrap();
        assert!(child.try_wait().unwrap().is_some());
        l.stop().await.unwrap();
    }

    #[tokio::test]
    async fn launch_attach_spawns_nothing() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        serve_200(listener);
        let target = Target::Attach {
            url: format!("http://127.0.0.1:{port}/"),
        };
        let mut l = launch(&target, None, Duration::from_secs(5)).await.unwrap();
        assert!(l.child.is_none());
        assert_eq!(l.url, web_url(&target));
        l.stop().await.unwrap();
    }

    #[tokio::test]
    async fn bind_free_ports_are_distinct_while_held() {
        let handles: Vec<_> = (0..64).map(|_| tokio::spawn(bind_free())).collect();
        let mut held = Vec::new();
        for h in handles {
            held.push(h.await.unwrap());
        }
        let mut ports: Vec<u16> = held.iter().map(|(_, p)| *p).collect();
        ports.sort_unstable();
        ports.dedup();
        assert_eq!(ports.len(), 64, "every held listener owns its own port");
        for (_, port) in &held {
            let err = TcpListener::bind(("127.0.0.1", *port)).await.unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
        }
    }

    #[tokio::test]
    async fn wait_ready_accepts_a_server_that_404s_at_root_and_200s_on_the_page_path() {
        let (listener, port) = bind_free().await;
        serve_by_path(listener, READY_PATH);
        wait_ready(
            &format!("http://127.0.0.1:{port}{READY_PATH}"),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        let r = wait_ready(
            &format!("http://127.0.0.1:{port}/"),
            Duration::from_millis(700),
        )
        .await;
        assert!(r.is_err(), "a 404 is not ready");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn launch_start_is_ready_when_only_the_page_path_answers_200() {
        let _exec = EXEC_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_binary(dir.path(), "fake", "exec sleep 30");
        let (listener, port) = bind_free().await;
        serve_by_path(listener, READY_PATH);
        let target = Target::Start { binary: bin, port };
        let mut l = launch(&target, None, Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(l.url, web_url(&target));
        assert!(l.url.ends_with('/'));
        assert!(l.child.is_some());
        l.stop().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn launch_start_times_out_when_only_root_answers_200() {
        let _exec = EXEC_LOCK.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_binary(dir.path(), "fake", "exec sleep 30");
        let (listener, port) = bind_free().await;
        serve_by_path(listener, "/");
        let target = Target::Start { binary: bin, port };
        let err = match launch(&target, None, Duration::from_millis(600)).await {
            Ok(_) => panic!("root alone must not be ready"),
            Err(e) => e,
        };
        assert!(err.contains("no 200"), "{err}");
    }

    #[test]
    fn ready_url_is_the_served_page_for_start_and_tunnel_and_the_given_url_for_attach() {
        let start = Target::Start {
            binary: "b".into(),
            port: 9123,
        };
        assert_eq!(
            ready_url(&start),
            "http://127.0.0.1:9123/team_page/file/index.html"
        );
        assert!(web_url(&start).ends_with('/'));
        let tunnel = Target::Tunnel {
            ssh: "ssh".into(),
            host: "h".into(),
            remote_port: 1,
            local_port: 9100,
        };
        assert_eq!(
            ready_url(&tunnel),
            "http://127.0.0.1:9100/team_page/file/index.html"
        );
        let attach = Target::Attach {
            url: "http://127.0.0.1:1/x".into(),
        };
        assert_eq!(ready_url(&attach), "http://127.0.0.1:1/x");
        assert_eq!(
            ready_url(&Target::Setup {
                binary: "b".into(),
                name: "n".into(),
                org: None
            }),
            ""
        );
    }

    #[test]
    fn ready_path_matches_the_teams_page_path() {
        assert_eq!(READY_PATH, "/team_page/file/index.html");
        let page = include_str!("../../team/src/page.rs");
        assert!(page.contains("PAGE_PATH: &str = \"/team_page/file/index.html\""));
    }

    #[tokio::test]
    async fn wait_ready_times_out_when_nothing_answers() {
        // Bound but never accepted: connects queue in the backlog and no
        // response ever comes, so every attempt times out.
        let (_held, port) = bind_free().await;
        let r = wait_ready(
            &format!("http://127.0.0.1:{port}/"),
            Duration::from_millis(300),
        )
        .await;
        assert!(r.is_err());
    }

    #[test]
    fn tunnel_args_forward_loopback_only() {
        let a = tunnel_args("box", 8765, 9100);
        assert_eq!(
            a,
            args(&[
                "-N",
                "-o",
                "ExitOnForwardFailure=yes",
                "-L",
                "127.0.0.1:9100:127.0.0.1:8765",
                "box"
            ])
        );
        assert_eq!(a.iter().filter(|x| *x == "-o").count(), 1);
        assert!(!a.iter().any(|x| x == "-i" || x.contains("key")));
    }

    #[test]
    fn tunnel_target_parses_host_and_port_and_refuses_urls() {
        match target_from_args(&args(&["--tunnel", "box:9000"])).unwrap() {
            Target::Tunnel {
                ssh,
                host,
                remote_port,
                local_port,
            } => {
                assert_eq!(ssh, DEFAULT_SSH);
                assert_eq!(host, "box");
                assert_eq!(remote_port, 9000);
                assert!(local_port > 0);
            }
            other => panic!("expected Tunnel, got {other:?}"),
        }
        match target_from_args(&args(&["--tunnel", "me@box.lan", "--ssh", "/x/ssh"])).unwrap() {
            Target::Tunnel {
                ssh,
                host,
                remote_port,
                ..
            } => {
                assert_eq!((ssh.as_str(), host.as_str()), ("/x/ssh", "me@box.lan"));
                assert_eq!(remote_port, DEFAULT_PORT);
            }
            other => panic!("expected Tunnel, got {other:?}"),
        }
        for bad in [
            "http://x@box",
            "a/b",
            "box?key=1",
            "-oProxyCommand=x",
            "box:0",
            "box:99999",
            "",
        ] {
            let err = target_from_args(&args(&["--tunnel", bad])).unwrap_err();
            assert!(
                bad.is_empty() || !err.contains(bad),
                "the error repeats `{bad}`: {err}"
            );
        }
        assert!(target_from_args(&args(&[
            "--tunnel",
            "box",
            "--attach",
            "http://127.0.0.1:1/"
        ]))
        .is_err());
        assert!(target_from_args(&args(&["--ssh", "ssh"])).is_err());
    }

    #[test]
    fn attach_refuses_userinfo_and_query_without_echoing_it() {
        for url in ["http://secret@127.0.0.1:1/", "http://127.0.0.1:1/?k=secret"] {
            let err = target_from_args(&args(&["--attach", url])).unwrap_err();
            assert!(!err.contains("secret"), "{err}");
            assert!(validate_loopback(url).is_err());
        }
        assert!(validate_loopback("http://127.0.0.1:1/#secret").is_err());
        assert!(validate_loopback("http://127.0.0.1:1/ok").is_ok());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn launch_tunnel_spawns_ssh_with_forward_and_returns_local_url() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("argv");
        let ssh = fake_binary(
            dir.path(),
            "fake_ssh",
            &format!("printf '%s\\n' \"$@\" > '{}'\nexec sleep 30", log.display()),
        );
        let (listener, local_port) = bind_free().await;
        serve_200(listener);
        let target = Target::Tunnel {
            ssh,
            host: "box".into(),
            remote_port: 8765,
            local_port,
        };
        let mut l = launch(&target, None, Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(l.url, format!("http://127.0.0.1:{local_port}/"));
        // the stand-in may not have run yet when the server already answers
        let mut text = String::new();
        for _ in 0..100 {
            text = std::fs::read_to_string(&log).unwrap_or_default();
            if text.lines().count() >= 6 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let seen: Vec<String> = text.lines().map(str::to_string).collect();
        assert_eq!(seen, tunnel_args("box", 8765, local_port));
        l.stop().await.unwrap();
        assert!(l.child.as_mut().unwrap().try_wait().unwrap().is_some());
        l.stop().await.unwrap();
    }

    #[test]
    fn setup_args_carry_no_secret() {
        assert_eq!(
            setup_args("my-app", Some("org")),
            args(&["setup", "github", "--name", "my-app", "--org", "org"])
        );
        assert_eq!(
            setup_args("my-app", None),
            args(&["setup", "github", "--name", "my-app"])
        );
        let t = target_from_args(&args(&["--setup-github", "my-app", "--org", "o-1"])).unwrap();
        assert_eq!(
            t,
            Target::Setup {
                binary: HEADLESS_BINARY.into(),
                name: "my-app".into(),
                org: Some("o-1".into())
            }
        );
        let long = "a".repeat(40);
        for bad in ["my app", "a=b", long.as_str(), ""] {
            assert!(
                target_from_args(&args(&["--setup-github", bad])).is_err(),
                "`{bad}` accepted"
            );
        }
        assert!(target_from_args(&args(&["--setup-github", "n", "--org", "a b"])).is_err());
        assert!(target_from_args(&args(&["--org", "o"])).is_err());
        for a in setup_args("my-app", Some("org")) {
            assert!(!a.contains("key") && !a.contains("token"), "{a}");
        }
    }

    #[test]
    fn setup_url_is_picked_from_child_output() {
        assert_eq!(
            setup_url("open this URL in your browser to create the app:"),
            None
        );
        assert_eq!(
            setup_url("http://127.0.0.1:5555/").as_deref(),
            Some("http://127.0.0.1:5555/")
        );
        assert_eq!(setup_url("http://example.com:5555/"), None);
        assert_eq!(setup_url("http://127.0.0.1.evil.com/"), None);
        assert_eq!(setup_url("http://127.0.0.1:5555/?k=1"), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn start_setup_forwards_args_and_returns_the_page_url() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("argv");
        let bin = fake_binary(
            dir.path(),
            "fake",
            &format!(
                "printf '%s\\n' \"$@\" > '{}'\necho 'open this URL in your browser to create the app:'\necho http://127.0.0.1:5555/\nexec sleep 30",
                log.display()
            ),
        );
        let mut l = start_setup(&bin, "my-app", Some("org"), Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(l.url, "http://127.0.0.1:5555/");
        let seen = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            seen.lines().collect::<Vec<_>>(),
            ["setup", "github", "--name", "my-app", "--org", "org"]
        );
        l.stop().await.unwrap();
        let quiet = fake_binary(dir.path(), "quiet", "echo nothing here");
        assert!(start_setup(&quiet, "n", None, Duration::from_secs(5))
            .await
            .is_err());
    }
}
