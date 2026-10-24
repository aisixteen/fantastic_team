//! App shell for the headless team binary's web UI (#37, part a).
//! No logic here: resolve a target, start or attach, show its URL.

use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};

/// The headless binary name (`[[bin]] name` in crates/team/Cargo.toml).
pub const HEADLESS_BINARY: &str = "fantastic_team";

/// The loopback port the packaged window loads (`app/tauri.conf.json`).
pub const DEFAULT_PORT: u16 = 8765;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Start { binary: String, port: u16 },
    Attach { url: String },
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

/// Parse `--attach <url>` / `--port <n>` / `--binary <path>`.
/// With no flags: start the headless binary on `DEFAULT_PORT`.
pub fn target_from_args(args: &[String]) -> Result<Target, String> {
    let mut attach = None;
    let mut port: Option<u16> = None;
    let mut binary = None;
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
            other => return Err(format!("unknown argument: {other}")),
        }
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

/// Attach URLs must be plain http on loopback (tunnel/relay = loopback).
pub fn validate_loopback(url: &str) -> Result<(), String> {
    let (auth, _) =
        split_http(url).ok_or_else(|| "attach URL must start with http://".to_string())?;
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
    let Target::Start { binary, port } = target else {
        wait_ready(&url, ready_timeout).await?;
        return Ok(Launched { child: None, url });
    };
    let mut cmd = Command::new(binary);
    cmd.args(start_args(*port))
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    if let Some(d) = dir {
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
        r = wait_ready(&url, ready_timeout) => Outcome::Ready(r),
    };
    match outcome {
        Outcome::Exited(status) => Err(match status {
            Ok(s) => format!("{binary} exited before serving {url}: {s}"),
            Err(e) => format!("{binary} exited before serving {url}: {e}"),
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
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_binary(dir.path(), "fake", "exec sleep 30");
        let port = free_port().unwrap();
        let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
        serve_200(listener);
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
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_binary(dir.path(), "fake", "exit 3");
        let port = free_port().unwrap();
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
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_binary(dir.path(), "fake", "exec sleep 30");
        let port = free_port().unwrap();
        let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
        serve_200(listener);
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
    async fn wait_ready_times_out_on_a_closed_port() {
        let port = free_port().unwrap();
        let r = wait_ready(
            &format!("http://127.0.0.1:{port}/"),
            Duration::from_millis(300),
        )
        .await;
        assert!(r.is_err());
    }
}
