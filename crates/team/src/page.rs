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
const CARD_JS: &str = include_str!("../web/card_connector.js");

/// The files the page serves, in order. Everything the page loads is one of these.
pub const FILES: [&str; 5] = ["index.html", "page.css", "shim.js", "page.js", "card.js"];

/// What `read_stream` declares on the record, so the host serves the page.
pub const READ_STREAM_DOC: &str = "args: path:str (req), offset:int?, length:int?. The dispatcher page (index.html, page.css, shim.js, page.js, card.js) as a BINARY reply: the bytes as the body, {path, offset, next_offset, eof, size} as the head.";

/// Cut the first `<tag>...</tag>` body out of `html`; the html gets `replacement` in its place.
fn cut(html: &str, open: &str, close: &str, replacement: &str) -> (String, String) {
    let start = html.find(open).expect("page has the tag");
    let body_at = start + open.len();
    let end = body_at + html[body_at..].find(close).expect("page closes the tag");
    let body = html[body_at..end].to_string();
    let out = format!(
        "{}{}{}",
        &html[..start],
        replacement,
        &html[end + close.len()..]
    );
    (out, body)
}

/// The dispatcher html cut into the page, its stylesheet and its script, so the page
/// has no inline style or script (the app's CSP allows `'self'` only).
fn split_page(html: &str) -> (String, String, String) {
    let (html, css) = cut(
        html,
        "<style>",
        "</style>",
        "<link rel=\"stylesheet\" href=\"page.css\">",
    );
    let (html, js) = cut(
        &html,
        "<script>",
        "</script>",
        "<script src=\"page.js\"></script>",
    );
    (html, css, js)
}

/// The page served as `index.html`: the dispatcher html with the shim script in its head.
pub fn index_html() -> String {
    split_page(dispatcher::HTML).0.replacen(
        "</head>",
        "<script src=\"shim.js\"></script>\n</head>",
        1,
    )
}

/// The bytes of one served file, or `None` for a path the page does not serve.
pub fn file_bytes(path: &str) -> Option<Vec<u8>> {
    Some(
        match path {
            "index.html" => index_html(),
            "page.css" => split_page(dispatcher::HTML).1,
            "shim.js" => SHIM.to_string(),
            "page.js" => split_page(dispatcher::HTML).2,
            "card.js" => CARD_JS.to_string(),
            _ => return None,
        }
        .into_bytes(),
    )
}

fn read_stream(payload: &Value) -> (Value, Option<Vec<u8>>) {
    let path = payload
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim_start_matches('/');
    let Some(bytes) = file_bytes(path) else {
        return (
            json!({"error": format!("team_page: no file {path:?} — this page serves {FILES:?}"),
                "reason": "not_found"}),
            None,
        );
    };
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
            "reflect" => json!({"id": agent_id.as_str(), "files": FILES}),
            "boot" => json!({"booted": true}),
            "list" => json!({"files": FILES}),
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
        let shim_at = html.find("<script src=\"shim.js\">").expect("the shim tag");
        let head_end = html.find("</head>").unwrap();
        assert!(shim_at < head_end);
        let shim = String::from_utf8(file_bytes("shim.js").unwrap()).unwrap();
        assert!(shim.contains("window.fantastic = {"));
        let js = String::from_utf8(file_bytes("page.js").unwrap()).unwrap();
        assert!(js.contains("function renderSidebar("));
        assert!(!html.contains("innerHTML") && !js.contains("innerHTML"));
    }

    #[test]
    fn index_page_has_the_agents_section() {
        let js = String::from_utf8(file_bytes("page.js").unwrap()).unwrap();
        assert!(js.contains("function renderAgents("));
        assert!(js.contains("'Agents'"));
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

    #[test]
    fn split_page_moves_style_and_script_out() {
        let (html, css, js) = split_page(dispatcher::HTML);
        assert!(!html.contains("<style>") && !html.contains("</style>"));
        assert_eq!(
            html.matches("<link rel=\"stylesheet\" href=\"page.css\">")
                .count(),
            1
        );
        assert_eq!(html.matches("<script src=\"page.js\"></script>").count(), 1);
        assert_eq!(html.matches("<script").count(), 1);
        assert!(!css.is_empty() && !js.is_empty());
        let rebuilt = html
            .replace(
                "<link rel=\"stylesheet\" href=\"page.css\">",
                &format!("<style>{css}</style>"),
            )
            .replace(
                "<script src=\"page.js\"></script>",
                &format!("<script>{js}</script>"),
            );
        assert_eq!(rebuilt, dispatcher::HTML);
    }

    fn whole(path: &str) -> (Value, Vec<u8>) {
        let (head, body) = read_stream(&json!({"path": path}));
        (head, body.unwrap_or_else(|| panic!("no file {path}")))
    }

    #[test]
    fn read_stream_serves_every_listed_file() {
        for f in ["page.css", "shim.js", "page.js", "card.js"] {
            let (head, body) = whole(f);
            assert!(!body.is_empty(), "{f}");
            assert_eq!(head["eof"], json!(true), "{f}");
        }
        let (_, shim) = whole("shim.js");
        assert!(String::from_utf8(shim)
            .unwrap()
            .contains("window.fantastic = {"));
    }

    #[tokio::test]
    async fn list_and_reflect_name_all_files() {
        let kernel = Arc::new(Kernel::new());
        let id = AgentId::from(PAGE_ID);
        for verb in ["list", "reflect"] {
            let r = PageBundle
                .handle(&id, &json!({"type": verb}), &kernel)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(r["files"], json!(FILES), "{verb}");
        }
    }

    #[test]
    fn card_connector_file_speaks_postmessage() {
        assert!(CARD_JS.contains("t: \"send\""));
        assert!(CARD_JS.contains("t: \"watch\""));
        assert!(CARD_JS.contains("postMessage"));
        assert!(dispatcher::HTML.contains("/team_page/file/card.js"));
        assert!(!dispatcher::HTML.contains("postMessage({ t: \"send\""));
    }
}
