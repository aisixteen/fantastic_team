//! The two address helpers of the team page: [`is_wildcard`] and [`free_port`].

/// A wildcard listener address: a browser cannot dial it.
pub fn is_wildcard(host: &str) -> bool {
    matches!(host, "0.0.0.0" | "::")
}

/// A port free on `host` now (the OS picks it), for a first run that names none.
pub fn free_port(host: &str) -> Result<u16, String> {
    std::net::TcpListener::bind((host, 0))
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| format!("no free port to pick: {e}"))
}

#[cfg(test)]
mod tests {
    use crate::test_boot::{boot_at, Seed};
    use crate::{hydrate, page};
    use fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
    use fantastic_kernel::{Item, Kernel};
    use serde_json::{json, Value};
    use std::path::Path;
    use std::sync::Arc;

    /// A port that is free right now (bound, read, released).
    fn free_port() -> u16 {
        // Never the same port twice in this process: the tests run in parallel and a
        // port released here may be handed out again before its test binds it.
        static ISSUED: std::sync::Mutex<Vec<u16>> = std::sync::Mutex::new(Vec::new());
        loop {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = l.local_addr().unwrap().port();
            let mut issued = ISSUED.lock().unwrap();
            if !issued.contains(&port) {
                issued.push(port);
                return port;
            }
        }
    }

    /// One HTTP/1.0 GET on 127.0.0.1:port; the whole response as text.
    async fn http_get(port: u16, path: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut conn = None;
        for _ in 0..50 {
            match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                Ok(c) => {
                    conn = Some(c);
                    break;
                }
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(40)).await,
            }
        }
        let mut c = conn.expect("the port never opened");
        let req = format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n");
        c.write_all(req.as_bytes()).await.unwrap();
        let mut out = Vec::new();
        c.read_to_end(&mut out).await.unwrap();
        String::from_utf8_lossy(&out).into_owned()
    }

    fn status_of(resp: &str) -> u16 {
        assert!(resp.starts_with("HTTP/1."), "{resp}");
        resp.split_whitespace().nth(1).unwrap().parse().unwrap()
    }

    /// Open a raw websocket on `/team_ws/ws`; the handshake head and the stream.
    async fn ws_open(port: u16) -> (String, tokio::net::TcpStream) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut c = loop {
            match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                Ok(c) => break c,
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(40)).await,
            }
        };
        let hs = format!(
            "GET /team_ws/ws HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        c.write_all(hs.as_bytes()).await.unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            let mut b = [0u8; 1];
            c.read_exact(&mut b).await.unwrap();
            head.push(b[0]);
        }
        (String::from_utf8_lossy(&head).into_owned(), c)
    }

    /// A websocket handshake, one `call` frame for `target`, and the `reply` frame.
    async fn ws_call(port: u16, target: &str, payload: Value) -> Value {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (head, mut c) = ws_open(port).await;
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");
        let msg =
            json!({"type": "call", "target": target, "payload": payload, "id": "p1"}).to_string();
        assert!(msg.len() < 126);
        let mut frame = vec![0x81, 0x80 | msg.len() as u8, 0, 0, 0, 0];
        frame.extend_from_slice(msg.as_bytes());
        c.write_all(&frame).await.unwrap();
        // The leg also announces what it handles as `event` frames: read to the reply.
        loop {
            let mut h2 = [0u8; 2];
            c.read_exact(&mut h2).await.unwrap();
            let len = match h2[1] & 0x7f {
                126 => {
                    let mut l = [0u8; 2];
                    c.read_exact(&mut l).await.unwrap();
                    u16::from_be_bytes(l) as usize
                }
                127 => {
                    let mut l = [0u8; 8];
                    c.read_exact(&mut l).await.unwrap();
                    u64::from_be_bytes(l) as usize
                }
                n => n as usize,
            };
            let mut body = vec![0u8; len];
            c.read_exact(&mut body).await.unwrap();
            let frame: Value = serde_json::from_slice(&body).unwrap();
            if frame["type"] != "event" {
                return frame;
            }
        }
    }

    /// The dispatcher, made with its own end of the page leg's edge (`callable_by`): the
    /// blueprint holds the leg's end (`team_ws.can_call`), the agent the person makes holds
    /// the other.
    async fn make_dispatcher(k: &Arc<Kernel>, groups: Value) {
        let mk = json!({"type": "create_agent", "id": "dispatcher", "parent_id": DEFAULT_ROOT_ID,
            "handler_module": crate::dispatcher::HANDLER_MODULE, "groups": groups,
            "callable_by": {page::WS_ID: ["sidebar", "tree", "open", "control"]}});
        let out = k.send(vec![Item::new(DEFAULT_ROOT_ID, mk)]).await;
        assert!(out[0].result.get("error").is_none(), "{}", out[0].result);
    }

    /// The page's socket leg reaches the dispatcher: a raw websocket handshake and one
    /// `call` frame for `sidebar`, answered by a `reply` frame with the same id.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_page_socket_reaches_the_dispatcher() {
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        let k = boot_at(dir.path(), Seed::Team, &[], Some(port)).await;
        make_dispatcher(&k, json!({})).await;
        let reply = ws_call(port, "dispatcher", json!({"type": "sidebar"})).await;
        assert_eq!(reply["type"], "reply", "{reply}");
        assert_eq!(reply["id"], "p1");
        assert!(reply["data"].get("error").is_none(), "{reply}");
        assert_eq!(reply["data"]["groups"], json!([]), "{reply}");
        k.stop().await;
    }

    /// The shell's first-run records: the `user` and, under it, the blueprint's store.
    const SEEDED_STORE: &str = ".fantastic/agents/user/agents/store/agent.yaml";

    #[tokio::test(flavor = "multi_thread")]
    async fn a_fresh_directory_serves_team_page_on_the_first_start() {
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        let k = boot_at(dir.path(), Seed::Team, &[], Some(port)).await;
        let resp = http_get(port, page::PAGE_PATH).await;
        assert_eq!(status_of(&resp), 200, "{resp}");
        let js = http_get(port, "/team_page/file/page.js").await;
        assert_eq!(status_of(&js), 200, "{js}");
        assert_eq!(
            status_of(&http_get(port, "/team_page/file/other.txt").await),
            404
        );
        assert!(
            dir.path().join(SEEDED_STORE).is_file(),
            "the seed was not taken away"
        );
        k.stop().await;
    }

    /// The page leg's end of every edge is the blueprint's: the seeded `team_ws` may call
    /// the dispatcher's page verbs and the `cadencer` and `user` verbs the team's page
    /// uses, nothing else and never `*`. The other end is on the agent the person makes
    /// (its own `callable_by`).
    #[tokio::test(flavor = "multi_thread")]
    async fn the_seeded_page_leg_may_call_exactly_its_blueprint_verbs() {
        let dir = tempfile::TempDir::new().unwrap();
        hydrate::seed(dir.path()).unwrap();
        let cc = read_can_call(dir.path());
        for (target, verbs) in [
            ("cadencer", json!(["status", "set"])),
            ("user", json!(["inquiries", "answer_inquiry"])),
        ] {
            assert_eq!(cc[target], verbs, "can_call {target}: {cc}");
        }
        assert_eq!(
            cc["dispatcher"],
            json!(["sidebar", "tree", "open", "control"])
        );
        for k in cc.as_object().unwrap().keys() {
            assert!(
                ["$self", "dispatcher", "cadencer", "user"].contains(&k.as_str()),
                "the seeded leg may call {k}: {cc}"
            );
            if k != "$self" {
                assert_ne!(cc[k], json!("*"), "{k}");
            }
        }
        for id in ["kernel", "kernel_state", "queue", "tasks"] {
            assert!(cc.get(id).is_none(), "{id}: {cc}");
        }
    }

    /// The seeded `team_ws` record's `can_call`, read from disk.
    fn read_can_call(dir: &Path) -> Value {
        let text =
            std::fs::read_to_string(dir.join(".fantastic/agents/web/agents/team_ws/agent.yaml"))
                .unwrap();
        let rec: Value = serde_yaml::from_str(&text).unwrap();
        rec["can_call"].clone()
    }

    /// fantastic refuses a web boot in which two children declare the same literal route
    /// (fantastic#446: "is already declared by"). The team's page and socket leg, mounted
    /// next to each other on the one host, must not clash.
    #[tokio::test(flavor = "multi_thread")]
    async fn team_web_routes_do_not_clash_at_boot() {
        let dir = tempfile::TempDir::new().unwrap();
        let port = free_port();
        // the boot of the host is the clash check: it must succeed and serve both routes
        let k = boot_at(dir.path(), Seed::Team, &[], Some(port)).await;
        let resp = http_get(port, page::PAGE_PATH).await;
        assert_eq!(status_of(&resp), 200, "{resp}");
        assert!(!resp.contains("is already declared by"), "{resp}");
        let (head, _c) = ws_open(port).await;
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");
        let web = k.agents.get(&fantastic_kernel::AgentId::from(page::WEB_ID));
        let status = web
            .map(|w| w.meta.read().unwrap().get("status").cloned())
            .unwrap_or(None);
        assert!(
            !status
                .map(|v| v.to_string().contains("already declared"))
                .unwrap_or(false),
            "the host recorded a route clash"
        );
        k.stop().await;
    }
}
