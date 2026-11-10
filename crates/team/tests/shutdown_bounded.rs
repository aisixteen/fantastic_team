//! #119: the team binary started `--headless` leaves the process on one SIGTERM (the
//! bounded stop is fantastic's). Unix only: the test sends a real signal.
#![cfg(unix)]

use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

mod common;
use common::{free_port, serial};

fn start(dir: &Path, port: u16) -> Child {
    Command::new(env!("CARGO_BIN_EXE_fantastic_team"))
        .args(["--headless", "--port", &port.to_string()])
        .current_dir(dir)
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

#[test]
fn sigterm_stops_a_headless_team() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let mut child = start(dir.path(), port);
    drop(wait_ready(&mut child, port));
    signal(&child, "-TERM");
    // the bound only guards against a hang
    wait_exit(&mut child, Duration::from_secs(30));
}
