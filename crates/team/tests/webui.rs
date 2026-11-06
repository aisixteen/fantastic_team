//! #224: `fantastic_team webui [--port N] [--host ADDR]` serves the team page from the
//! binary, on a real kernel: the page and its socket answer on the port, `--host` is the
//! address the web host binds, a kernel already running refuses the word, and `webui`
//! with `canvas` is refused before anything is written. Unix only: the tests send signals.
#![cfg(unix)]

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

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

/// A running binary; stopped (SIGTERM, then reaped) when dropped.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .args(["-TERM", &self.0.id().to_string()])
            .status();
        let end = Instant::now() + Duration::from_secs(10);
        while self.0.try_wait().ok().flatten().is_none() {
            if Instant::now() > end {
                let _ = self.0.kill();
                let _ = self.0.wait();
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

fn start(dir: &Path, args: &[&str]) -> Running {
    Running(
        Command::new(env!("CARGO_BIN_EXE_fantastic_team"))
            .args(args)
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    )
}

/// How long a command that is refused may take to end.
const REFUSAL_LIMIT: Duration = Duration::from_secs(60);

fn run_to_end(dir: &Path, args: &[&str]) -> Output {
    run_to_end_within(dir, args, REFUSAL_LIMIT)
}

/// Run the binary to its end, judged over `limit`, never waited on forever (#241). A
/// command that is still running at the limit is killed and named: a refused second
/// `webui` that got past the lock check became the directory's running team. A
/// command that ended while its output stays open is named too: something it left
/// behind holds its stdout or stderr. `Command::output` would hang on either.
fn run_to_end_within(dir: &Path, args: &[&str], limit: Duration) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fantastic_team"));
    cmd.args(args).current_dir(dir);
    run_within(cmd, limit)
}

/// [`run_to_end_within`] for any command.
fn run_within(mut cmd: Command, limit: Duration) -> Output {
    let args: Vec<String> = std::iter::once(cmd.get_program())
        .chain(cmd.get_args())
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let drain = |mut pipe: Box<dyn Read + Send>| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            let _ = tx.send(buf);
        });
        rx
    };
    let stdout = drain(Box::new(child.stdout.take().unwrap()));
    let stderr = drain(Box::new(child.stderr.take().unwrap()));
    let pid = child.id();
    let (tx, exited) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait());
    });
    let end = Instant::now() + limit;
    let left = || end.saturating_duration_since(Instant::now());
    let Ok(status) = exited.recv_timeout(left()) else {
        let _ = Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status();
        panic!(
            "{args:?} was still running after {limit:?}: it did not end, it became a running team"
        );
    };
    let status = status.unwrap();
    let out = |rx: std::sync::mpsc::Receiver<Vec<u8>>, name: &str| {
        rx.recv_timeout(left()).unwrap_or_else(|_| {
            panic!("{args:?} ended ({status}) but its {name} stayed open past {limit:?}: a process it left holds it")
        })
    };
    let stdout = out(stdout, "stdout");
    let stderr = out(stderr, "stderr");
    Output {
        status,
        stdout,
        stderr,
    }
}

/// One HTTP/1.0 GET; the status and the whole response, or None while nothing listens.
fn get(port: u16, path: &str) -> Option<(u16, String)> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut c = TcpStream::connect_timeout(&addr, Duration::from_millis(500)).ok()?;
    c.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let req = format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n");
    c.write_all(req.as_bytes()).ok()?;
    let mut buf = Vec::new();
    c.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    let status = text.split_whitespace().nth(1)?.parse().ok()?;
    Some((status, text))
}

const PAGE: &str = "/team_page/file/index.html";

fn wait_page(child: &mut Running, port: u16) -> String {
    wait_page_within(child, port, Duration::from_secs(60))
}

/// Poll the page until it answers 200; panics when the process exits or `limit` passes.
/// One `get` is one try: a process that is up and listening can be slow to accept for a
/// moment on a loaded machine (#241), so "answers" is judged over `limit`, not one try.
fn wait_page_within(child: &mut Running, port: u16, limit: Duration) -> String {
    let end = Instant::now() + limit;
    loop {
        if let Some(st) = child.0.try_wait().unwrap() {
            panic!("the binary exited with {st} before the page answered on {port}");
        }
        if let Some((200, text)) = get(port, PAGE) {
            return text;
        }
        assert!(Instant::now() < end, "the page never answered on {port}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A websocket handshake on `/team_ws/ws` and one `call` frame; the handshake head and
/// the first non-`event` frame.
fn ws_call(port: u16, target: &str, payload: serde_json::Value) -> (String, serde_json::Value) {
    let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let hs = format!(
        "GET /team_ws/ws HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    c.write_all(hs.as_bytes()).unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut b = [0u8; 1];
        c.read_exact(&mut b).unwrap();
        head.push(b[0]);
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    // What follows a refused upgrade is an HTTP body, not frames (#241: a 404 from a
    // host without the socket leg read as a frame and failed the JSON parse).
    assert!(
        head.starts_with("HTTP/1.1 101"),
        "no upgrade on {port}: {head}"
    );
    let msg = serde_json::json!({"type": "call", "target": target, "payload": payload, "id": "p1"})
        .to_string();
    assert!(msg.len() < 126);
    let mut frame = vec![0x81, 0x80 | msg.len() as u8, 0, 0, 0, 0];
    frame.extend_from_slice(msg.as_bytes());
    c.write_all(&frame).unwrap();
    loop {
        let mut h2 = [0u8; 2];
        c.read_exact(&mut h2).unwrap();
        let len = match h2[1] & 0x7f {
            126 => {
                let mut l = [0u8; 2];
                c.read_exact(&mut l).unwrap();
                u16::from_be_bytes(l) as usize
            }
            127 => {
                let mut l = [0u8; 8];
                c.read_exact(&mut l).unwrap();
                u64::from_be_bytes(l) as usize
            }
            n => n as usize,
        };
        let mut body = vec![0u8; len];
        c.read_exact(&mut body).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if v["type"] != "event" {
            return (head, v);
        }
    }
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn webui_port_serves_the_team_page_and_its_socket() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let mut team = start(dir.path(), &["webui", "--port", &port.to_string()]);
    let page = wait_page(&mut team, port);
    assert!(
        page.contains("<html") || page.contains("<!DOCTYPE"),
        "{page}"
    );
    // the page's files, and the canvas is not there
    let (js, _) = get(port, "/team_page/file/page.js").unwrap();
    assert_eq!(js, 200);
    let (canvas, _) = get(port, "/canvas_page/file/index.html").unwrap();
    assert_eq!(canvas, 404, "webui serves the team page, not the canvas");
    // the socket: the handshake and a call answered under the call's id
    let (head, reply) = ws_call(port, "dispatcher", serde_json::json!({"type": "sidebar"}));
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    assert_eq!(reply["id"], "p1", "{reply}");
    // the records the page needs are on disk: the host, and the page and socket under it
    let web = dir.path().join(".fantastic/agents/web");
    for rec in [
        "agent.yaml",
        "agents/team_page/agent.yaml",
        "agents/team_ws/agent.yaml",
    ] {
        assert!(web.join(rec).is_file(), "{rec} missing under {web:?}");
    }
}

#[test]
fn webui_announces_the_page_it_serves() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let mut team = start(dir.path(), &["webui", "--port", &port.to_string()]);
    wait_page(&mut team, port);
    let mut out = team.0.stdout.take().unwrap();
    let _ = Command::new("kill")
        .args(["-TERM", &team.0.id().to_string()])
        .status();
    let mut seen = String::new();
    let mut buf = [0u8; 512];
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        match out.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => seen.push_str(&String::from_utf8_lossy(&buf[..n])),
        }
    }
    assert!(
        seen.contains(&format!("the team page: http://127.0.0.1:{port}{PAGE}")),
        "{seen}"
    );
}

#[test]
fn host_is_the_address_the_web_host_binds() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let mut team = start(
        dir.path(),
        &["webui", "--port", &port.to_string(), "--host", "0.0.0.0"],
    );
    wait_page(&mut team, port);
    let rec = std::fs::read_to_string(dir.path().join(".fantastic/agents/web/agent.yaml")).unwrap();
    assert!(
        rec.contains("0.0.0.0"),
        "the web record names its bind: {rec}"
    );
}

#[test]
fn a_host_that_is_not_an_address_and_a_bad_port_are_refused() {
    let _s = serial();
    for (args, want) in [
        (
            &["webui", "--host", "localhost"][..],
            "--host must be an IP address, got `localhost`",
        ),
        (
            &["webui", "--port", "0"],
            "--port must be 1..65535, got `0`",
        ),
        (&["webui", "--port"], "--port needs a number"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let o = run_to_end(dir.path(), args);
        assert_eq!(o.status.code(), Some(2), "{args:?}: {}", text(&o.stderr));
        assert!(
            text(&o.stderr).contains(want),
            "{args:?}: {}",
            text(&o.stderr)
        );
        assert!(
            !dir.path().join(".fantastic").exists(),
            "{args:?}: a refused command writes nothing"
        );
    }
}

#[test]
fn webui_and_canvas_together_are_refused_before_anything_is_written() {
    let _s = serial();
    for args in [
        &["webui", "--port", "8123", "canvas"][..],
        &["canvas", "--port", "8123", "webui"],
    ] {
        let dir = tempfile::tempdir().unwrap();
        let o = run_to_end(dir.path(), args);
        assert_eq!(o.status.code(), Some(2), "{args:?}: {}", text(&o.stderr));
        let err = text(&o.stderr);
        assert!(
            err.contains("`webui` and `canvas` cannot run together"),
            "{args:?}: {err}"
        );
        assert!(!dir.path().join(".fantastic").exists(), "{args:?}");
    }
}

#[test]
fn a_kernel_already_running_here_refuses_webui() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let mut team = start(dir.path(), &["webui", "--port", &port.to_string()]);
    wait_page(&mut team, port);
    let o = run_to_end(dir.path(), &["webui", "--port", &free_port().to_string()]);
    assert_eq!(o.status.code(), Some(2), "{}", text(&o.stderr));
    assert!(
        text(&o.stderr).contains("already running in this directory"),
        "{}",
        text(&o.stderr)
    );
    assert!(
        team.0.try_wait().unwrap().is_none(),
        "the running team is untouched"
    );
    // The refusal wrote nothing into the running team's records: its web host still
    // names its own port, not the refused command's.
    let rec = std::fs::read_to_string(dir.path().join(".fantastic/agents/web/agent.yaml")).unwrap();
    assert!(rec.contains(&format!("port: {port}")), "{rec}");
    // And the team still serves. The refused start is over; the question is whether the
    // running team answers, which `wait_page_within` asks until it does or the limit
    // passes (#241: a single try raced the team's own accept on a loaded machine).
    wait_page_within(&mut team, port, Duration::from_secs(30));
}

#[test]
fn headless_over_webui_records_opens_no_port() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let mut team = start(dir.path(), &["webui", "--port", &port.to_string()]);
    wait_page(&mut team, port);
    drop(team); // stopped and reaped: the folder keeps the page's records
    assert!(dir
        .path()
        .join(".fantastic/agents/web/agents/team_page/agent.yaml")
        .is_file());
    let mut headless = start(dir.path(), &["--headless"]);
    // give the kernel time to boot every record, then ask the port
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end {
        assert!(
            headless.0.try_wait().unwrap().is_none(),
            "--headless exited"
        );
        assert!(
            get(port, PAGE).is_none(),
            "--headless served the page that webui recorded"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// #241: a listener that is up but accepts late is answered by `wait_page_within`, which
/// judges over its limit; one that never answers fails at the limit, and a process that
/// exited fails at once. (A single `get` against the late listener has already given up.)
#[test]
fn wait_page_within_rides_out_a_late_accept_and_fails_when_nothing_answers() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
    let mut team = start(dir.path(), &["--headless"]);
    // the listener is bound but does not accept for a while
    let late = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1500));
        let (mut c, _) = listener.accept().unwrap();
        let mut req = [0u8; 512];
        let _ = c.read(&mut req);
        let _ = c.write_all(b"HTTP/1.0 200 OK\r\n\r\nok");
    });
    let body = wait_page_within(&mut team, port, Duration::from_secs(20));
    assert!(body.ends_with("ok"), "{body}");
    late.join().unwrap();
    // nothing answers on a bound port that never accepts: the limit is the failure
    let silent = TcpListener::bind("127.0.0.1:0").unwrap();
    let silent_port = silent.local_addr().unwrap().port();
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_page_within(&mut team, silent_port, Duration::from_millis(800))
    }));
    assert!(
        failed.is_err(),
        "a port that never answers must fail at the limit"
    );
}

/// #241: `run_to_end` judges over its limit. A command that keeps running (a kernel,
/// what a refused `webui` became when it got past the lock check) fails at the limit
/// and is killed, not waited on forever; one that ends is returned whole.
#[test]
fn run_to_end_fails_a_command_that_keeps_running_and_kills_it() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let failed = std::panic::catch_unwind(|| {
        run_to_end_within(dir.path(), &["--headless"], Duration::from_secs(3))
    });
    let msg = failed.expect_err("a kernel that runs on must fail at the limit");
    let msg = msg.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(msg.contains("still running"), "{msg}");
    // killed: its lock on the directory is gone with it
    let end = Instant::now() + Duration::from_secs(10);
    while fantastic_kernel::lock::held(dir.path()).unwrap().is_some() {
        assert!(Instant::now() < end, "the kernel was left running");
        std::thread::sleep(Duration::from_millis(50));
    }
    let o = run_to_end_within(
        dir.path(),
        &["webui", "--port", "0"],
        Duration::from_secs(30),
    );
    assert_eq!(o.status.code(), Some(2), "{}", text(&o.stderr));
    // a command that ends while a process it started holds its output is named too
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "sleep 30 & echo started"]);
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_within(cmd, Duration::from_secs(3))
    }));
    let msg = failed.expect_err("an output held open must fail at the limit");
    let msg = msg.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(
        msg.contains("ended (exit status: 0) but its stdout stayed open"),
        "{msg}"
    );
}
