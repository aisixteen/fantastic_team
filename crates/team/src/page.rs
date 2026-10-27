//! The page agent (`team_page.tools`): the dispatcher's html served by the `web`
//! host on `--port`, the way the canvas page is. The host's `GET /<id>/file/<path>`
//! route pipes any agent that answers `read_stream`, so `index.html` is the
//! dispatcher page with a small shim that gives it the `fantastic.send` and
//! `fantastic.watch` calls over the team's `web_ws` leg. Read-only.

use std::sync::Arc;

use async_trait::async_trait;
use fantastic_kernel::bundle::{Bundle, BundleError, Reply};
use fantastic_kernel::{AgentId, Kernel};
use serde_json::{json, Value};

use crate::dispatcher;

/// The handler module this bundle registers under.
pub const HANDLER_MODULE: &str = "team_page.tools";
/// The page agent's id, under the `web` host.
pub const PAGE_ID: &str = "team_page";
/// The web_ws leg's id, under the `web` host; the page's shim connects to `/team_ws/ws`.
pub const WS_ID: &str = "team_ws";
/// The host's id.
pub const WEB_ID: &str = "web";
/// The page's path under the host.
pub const PAGE_PATH: &str = "/team_page/file/index.html";

const SHIM: &str = include_str!("../web/page_shim.js");

/// What `read_stream` declares on the record, so the host serves the page.
pub const READ_STREAM_DOC: &str = "args: path:str (req), offset:int?, length:int?. The dispatcher page as a BINARY reply: the bytes as the body, {path, offset, next_offset, eof, size} as the head.";

/// The page served as `index.html`: the dispatcher html with the shim in its head.
pub fn index_html() -> String {
    dispatcher::HTML.replacen("</head>", &format!("<script>\n{SHIM}</script>\n</head>"), 1)
}

fn read_stream(payload: &Value) -> (Value, Option<Vec<u8>>) {
    let path = payload
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim_start_matches('/');
    if path != "index.html" {
        return (
            json!({"error": format!("team_page: no file {path:?} — this page serves [\"index.html\"]"),
                "reason": "not_found"}),
            None,
        );
    }
    let bytes = index_html().into_bytes();
    let size = bytes.len() as u64;
    let offset = payload
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(size);
    let length = payload
        .get("length")
        .and_then(Value::as_u64)
        .unwrap_or(size - offset);
    let end = offset.saturating_add(length).min(size);
    (
        json!({"path": path, "offset": offset, "next_offset": end, "eof": end >= size, "size": size}),
        Some(bytes[offset as usize..end as usize].to_vec()),
    )
}

/// The bundle. Stateless: the page is in the binary.
#[derive(Default)]
pub struct PageBundle;

#[async_trait]
impl Bundle for PageBundle {
    fn name(&self) -> &str {
        "team_page"
    }

    fn verb_docs(&self) -> Value {
        json!({
            "reflect": "No args.",
            "boot": "No-op; the parent `web` host serves the route.",
            "read_stream": READ_STREAM_DOC,
            "list": "No args. The served paths.",
        })
    }

    async fn handle(
        &self,
        agent_id: &AgentId,
        payload: &Value,
        _kernel: &Arc<Kernel>,
    ) -> Result<Reply, BundleError> {
        let verb = payload.get("type").and_then(Value::as_str).unwrap_or("");
        Ok(Some(match verb {
            "reflect" => json!({"id": agent_id.as_str(), "files": ["index.html"]}),
            "boot" => json!({"booted": true}),
            "list" => json!({"files": ["index.html"]}),
            "read_stream" => read_stream(payload).0,
            other => {
                json!({"error": format!("team_page: unknown type {other:?}"), "reason": "unhandled", "verb": other})
            }
        }))
    }

    async fn handle_binary(
        &self,
        agent_id: &AgentId,
        header: Value,
        _body: Option<fantastic_kernel::Blob>,
        kernel: &Arc<Kernel>,
    ) -> Result<(Reply, Option<fantastic_kernel::Blob>), BundleError> {
        if header.get("type").and_then(Value::as_str) == Some("read_stream") {
            let (head, body) = read_stream(&header);
            return Ok((
                Some(head),
                body.map(|b| fantastic_kernel::Blob::key("bytes", b)),
            ));
        }
        Ok((self.handle(agent_id, &header, kernel).await?, None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_is_the_dispatcher_with_the_shim_in_its_head() {
        let html = index_html();
        let shim_at = html.find("window.fantastic = {").expect("the shim");
        let head_end = html.find("</head>").unwrap();
        assert!(shim_at < head_end);
        assert!(html.contains("function renderSidebar("));
        assert!(!html.contains("innerHTML"));
    }

    #[test]
    fn index_page_has_the_agents_section() {
        let html = index_html();
        assert!(html.contains("function renderAgents("));
        assert!(html.contains("'Agents'"));
    }

    #[test]
    fn read_stream_serves_index_in_chunks_and_refuses_other_paths() {
        let (head, body) = read_stream(&json!({"path": "index.html", "length": 10}));
        assert_eq!(body.unwrap().len(), 10);
        assert_eq!(head["eof"], json!(false));
        assert_eq!(head["next_offset"], json!(10));
        let (head, body) = read_stream(&json!({"path": "/index.html", "offset": 10}));
        assert_eq!(head["eof"], json!(true));
        assert_eq!(
            body.unwrap().len() as u64,
            head["size"].as_u64().unwrap() - 10
        );
        let (head, body) = read_stream(&json!({"path": "secret.txt"}));
        assert!(body.is_none());
        assert_eq!(head["reason"], "not_found");
    }

    #[test]
    fn the_shim_speaks_the_ws_frames_and_uses_the_leg_id() {
        for needle in [
            "type: 'call'",
            "type: 'watch'",
            "type: 'unwatch'",
            "'event'",
            "/team_ws/ws",
        ] {
            assert!(SHIM.contains(needle), "{needle}");
        }
        assert!(!SHIM.contains("innerHTML"));
    }
}
