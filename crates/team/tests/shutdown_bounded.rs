//! #119: the team binary leaves the process within a few seconds of SIGTERM or
//! SIGINT, even with an agent mid-model-call (a backend that never answers) or
//! a connected websocket client. Unix only: the tests send real signals.
#![cfg(unix)]

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

const LIMIT: Duration = Duration::from_secs(5);

/// Every test holds this for its whole body, so two tests never race for a
/// port, a child process or a stray listener.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// Ports already handed out in this test process; `free_port` never repeats one.
static ISSUED: Mutex<BTreeSet<u16>> = Mutex::new(BTreeSet::new());

fn free_port() -> u16 {
    let mut issued = ISSUED.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        if issued.insert(port) {
            return port;
        }
    }
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

/// Wait until `port` accepts a connection and return that connection. Checks
/// the child first on every pass, so a bind failure or early exit panics with
/// its status instead of connecting to someone else's listener.
fn wait_ready(child: &mut Child, port: u16) -> TcpStream {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(st) = child.try_wait().unwrap() {
            panic!("child exited with {st} before port {port} accepted");
        }
        if let Ok(stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) {
            return stream;
        }
        assert!(Instant::now() < end, "port {port} never accepted");
        std::thread::sleep(Duration::from_millis(50));
    }
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
    let mut child = start(dir, port);
    drop(wait_ready(&mut child, port));
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
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let mut child = start(dir.path(), port);
    drop(wait_ready(&mut child, port));
    signal(&child, "-TERM");
    wait_exit(&mut child, LIMIT);
}

#[test]
fn sigterm_exits_within_5s_with_inflight_model_call() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let (mut child, mut pending) = start_with_inflight_call(dir.path());
    signal(&child, "-TERM");
    wait_exit(&mut child, LIMIT);
    let _ = pending.kill();
    let _ = pending.wait();
}

#[test]
fn sigint_exits_within_5s_with_inflight_model_call() {
    let _serial = serial();
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
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let mut child = start(dir.path(), port);
    let mut ws = wait_ready(&mut child, port);
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
    let _serial = serial();
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
    drop(wait_ready(&mut again, port));
    signal(&again, "-TERM");
    wait_exit(&mut again, LIMIT);
}

/// Regression for the parallel port race: no two `free_port` calls in one test
/// process may hand out the same number, even when called concurrently.
#[test]
fn free_port_never_repeats_within_process() {
    let handles: Vec<_> = (0..8)
        .map(|_| std::thread::spawn(|| (0..25).map(|_| free_port()).collect::<Vec<u16>>()))
        .collect();
    let mut ports: Vec<u16> = handles
        .into_iter()
        .flat_map(|h| h.join().unwrap())
        .collect();
    let total = ports.len();
    ports.sort_unstable();
    ports.dedup();
    assert_eq!(ports.len(), total, "free_port handed out a duplicate port");
}

/// Regression for the wrong-listener race: when the child has already exited,
/// `wait_ready` must report the exit even though the port accepts (here a
/// listener held by this test stands in for another test's binary).
#[test]
fn wait_ready_panics_when_child_exits_before_listening() {
    let _serial = serial();
    let held = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port();
    let mut child = Command::new("sh")
        .args(["-c", "exit 3"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child.wait().unwrap();
    let start = Instant::now();
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = wait_ready(&mut child, port);
    }));
    assert!(start.elapsed() < Duration::from_secs(5));
    let payload = res.expect_err("wait_ready returned Ok for an exited child");
    let msg = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    assert!(msg.contains("exit status"), "panic message was: {msg}");
    drop(held);
}
