//! #119: the team binary leaves the process within a few seconds of SIGTERM or
//! SIGINT, even with an agent mid-model-call (a backend that never answers) or
//! a connected websocket client. Unix only: the tests send real signals.
#![cfg(unix)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const LIMIT: Duration = Duration::from_secs(5);

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn start(dir: &Path, port: u16) -> Child {
    Command::new(env!("CARGO_BIN_EXE_fantastic_team"))
        .args(["--port", &port.to_string(), "--dir"])
        .arg(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn wait_port(port: u16) {
    let end = Instant::now() + Duration::from_secs(30);
    while Instant::now() < end {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("port {port} never accepted");
}

fn signal(child: &Child, sig: &str) {
    let st = Command::new("kill")
        .args([sig, &child.id().to_string()])
        .status()
        .unwrap();
    assert!(st.success());
}

/// Poll `try_wait` until the child exits or `limit` passes (then kill it, fail).
fn wait_exit(child: &mut Child, limit: Duration) -> ExitStatus {
    let end = Instant::now() + limit;
    loop {
        if let Some(st) = child.try_wait().unwrap() {
            return st;
        }
        if Instant::now() > end {
            let _ = child.kill();
            let _ = child.wait();
            panic!("process still running {limit:?} after the signal");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A backend that accepts connections and never answers.
fn mock_backend() -> (u16, std::thread::JoinHandle<()>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let h = std::thread::spawn(move || {
        let mut held = Vec::new();
        for s in l.incoming().flatten() {
            held.push(s);
        }
    });
    (port, h)
}

fn send(dir: &Path, item: &str) -> Child {
    Command::new(env!("CARGO_BIN_EXE_fantastic_team"))
        .args(["send", item])
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

/// Start the binary, add an AI agent pointed at a backend that never answers,
/// and send it a message so a model call is in flight (the `send` call waits
/// for the reply, so it is left running and reaped at the end).
fn start_with_inflight_call(dir: &Path) -> (Child, Child) {
    let (backend, _h) = mock_backend();
    let port = free_port();
    let child = start(dir, port);
    wait_port(port);
    let create = serde_json::json!({"target": "kernel_state", "payload": {
        "type": "create_agent", "parent_id": "kernel_state", "id": "lead",
        "handler_module": "vllm_backend.tools", "model": "m",
        "endpoint": format!("http://127.0.0.1:{backend}/v1"),
        "memory_store_id": "memory", "kv_overflow": "evict",
        "max_tokens": 100, "context_window": 8000, "kv_slots": 1}});
    // The kernel takes a moment to be reachable for `send` after the port opens.
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        if send(dir, &create.to_string()).wait().unwrap().success() {
            break;
        }
        assert!(Instant::now() < end, "create_agent never succeeded");
        std::thread::sleep(Duration::from_millis(300));
    }
    let item = r#"{"target":"lead","payload":{"type":"send","text":"hello"}}"#;
    let pending = send(dir, item);
    // Let the model call start.
    std::thread::sleep(Duration::from_millis(1500));
    (child, pending)
}

#[test]
fn sigterm_exits_within_5s_idle() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let mut child = start(dir.path(), port);
    wait_port(port);
    signal(&child, "-TERM");
    wait_exit(&mut child, LIMIT);
}

#[test]
fn sigterm_exits_within_5s_with_inflight_model_call() {
    let dir = tempfile::tempdir().unwrap();
    let (mut child, mut pending) = start_with_inflight_call(dir.path());
    signal(&child, "-TERM");
    wait_exit(&mut child, LIMIT);
    let _ = pending.kill();
    let _ = pending.wait();
}

#[test]
fn sigint_exits_within_5s_with_inflight_model_call() {
    let dir = tempfile::tempdir().unwrap();
    let (mut child, mut pending) = start_with_inflight_call(dir.path());
    signal(&child, "-INT");
    let st = wait_exit(&mut child, LIMIT);
    let _ = pending.kill();
    let _ = pending.wait();
    // The graceful stop cannot finish with a call in flight: the watchdog exits.
    assert_eq!(st.code(), Some(130), "unexpected status {st:?}");
}

/// Guard: may already pass without the watchdog (the page grace is 5 s but
/// the idle client needs no stop).
#[test]
fn sigterm_exits_within_5s_with_connected_ws_client() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let mut child = start(dir.path(), port);
    wait_port(port);
    let mut ws = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    ws.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    ws.write_all(
        format!(
            "GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        .as_bytes(),
    )
    .unwrap();
    let mut buf = [0u8; 256];
    let _ = ws.read(&mut buf);
    signal(&child, "-TERM");
    wait_exit(&mut child, LIMIT);
    drop(ws);
}

#[test]
fn forced_exit_releases_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let (mut child, mut pending) = start_with_inflight_call(dir.path());
    signal(&child, "-TERM");
    wait_exit(&mut child, LIMIT);
    let _ = pending.kill();
    let _ = pending.wait();
    assert!(
        !dir.path().join(".fantastic/lock.yaml").exists(),
        "lock file left behind"
    );
    // A second start in the same dir succeeds.
    let port = free_port();
    let mut again = start(dir.path(), port);
    wait_port(port);
    signal(&again, "-TERM");
    wait_exit(&mut again, LIMIT);
}
