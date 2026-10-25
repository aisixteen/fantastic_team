//! The HTTP edge of the pack registry: fetch a hosted signed index, download a
//! pack's files, and POST a submission, a report or an updated index to the
//! registry. Nothing here decides anything: the index is parsed and verified by
//! [`pack_registry`], the downloaded files are checked against the pinned hash
//! by `pack_bundle::verify_download`. A request carries no credentials. Only
//! https is used; the `*_with` forms take `allow_insecure` so tests can talk to
//! a local mock, and no public function passes `true`.
//!
//! An error names the status or the cause, never the response body.

use std::time::Duration;

use serde_json::Value;

use crate::pack_registry::{Entry, Index};

/// Request timeout.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Largest index document read.
const MAX_INDEX: usize = 4 * 1024 * 1024;
/// Largest pack document read.
pub const MAX_PACK: usize = 16 * 1024 * 1024;

/// The files of a downloaded pack, as `(relative path, utf8 text)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackDoc {
    /// Every file in the document, `pack.yaml` included.
    pub files: Vec<(String, String)>,
}

fn check_scheme(url: &str, allow_insecure: bool) -> Result<(), String> {
    if url.starts_with("https://") || (allow_insecure && url.starts_with("http://")) {
        Ok(())
    } else {
        Err("registry url must be https".to_string())
    }
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "could not build the http client".to_string())
}

async fn read_capped(mut resp: reqwest::Response, cap: usize) -> Result<String, String> {
    if !resp.status().is_success() {
        return Err(format!("registry answered HTTP {}", resp.status().as_u16()));
    }
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|_| "registry read failed".to_string())?
    {
        if body.len() + chunk.len() > cap {
            return Err(format!("registry document is over {cap} bytes"));
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|_| "registry document is not utf-8".to_string())
}

async fn get_text(url: &str, allow_insecure: bool, cap: usize) -> Result<String, String> {
    check_scheme(url, allow_insecure)?;
    let resp = client()?
        .get(url)
        .header("User-Agent", "fantastic_team")
        .send()
        .await
        .map_err(|_| "could not reach the registry".to_string())?;
    read_capped(resp, cap).await
}

/// Fetch and verify a hosted index. A document that is not an index is an
/// error; a bad entry is skipped and reported in `skipped`.
pub async fn fetch_index(url: &str, trusted_keys: &[String]) -> Result<Index, String> {
    fetch_index_with(url, trusted_keys, false).await
}

/// [`fetch_index`] with the scheme check relaxed for tests.
pub async fn fetch_index_with(
    url: &str,
    trusted_keys: &[String],
    allow_insecure: bool,
) -> Result<Index, String> {
    let text = get_text(url, allow_insecure, MAX_INDEX).await?;
    Index::parse(&text, trusted_keys)
}

/// The index document as text, for callers that keep the raw text.
pub async fn fetch_index_text(url: &str, allow_insecure: bool) -> Result<String, String> {
    get_text(url, allow_insecure, MAX_INDEX).await
}

/// Parse a pack document `{"files": {"<path>": "<text>"}}`.
pub fn parse_pack_doc(text: &str) -> Result<PackDoc, String> {
    let v: Value = serde_json::from_str(text).map_err(|_| "pack is not json".to_string())?;
    let files = v
        .get("files")
        .and_then(Value::as_object)
        .ok_or("pack document needs a `files` object")?;
    let mut out = Vec::with_capacity(files.len());
    for (path, body) in files {
        let body = body
            .as_str()
            .ok_or_else(|| format!("pack file `{path}` must be text"))?;
        out.push((path.clone(), body.to_string()));
    }
    Ok(PackDoc { files: out })
}

/// Download the pack an index entry points to. The files are NOT trusted yet:
/// check them with `pack_bundle::verify_download` against the entry's hash.
pub async fn download_pack(entry: &Entry) -> Result<PackDoc, String> {
    download_pack_with(entry, false, MAX_PACK).await
}

/// [`download_pack`] with the scheme check and the size cap set by the caller.
pub async fn download_pack_with(
    entry: &Entry,
    allow_insecure: bool,
    cap: usize,
) -> Result<PackDoc, String> {
    let text = get_text(&entry.url, allow_insecure, cap).await?;
    parse_pack_doc(&text)
}

/// POST a JSON body (no auth headers) and return the JSON reply, or `{}` for
/// an empty body.
pub async fn post_json(url: &str, body: &Value, allow_insecure: bool) -> Result<Value, String> {
    check_scheme(url, allow_insecure)?;
    let resp = client()?
        .post(url)
        .header("User-Agent", "fantastic_team")
        .json(body)
        .send()
        .await
        .map_err(|_| "could not reach the registry".to_string())?;
    let text = read_capped(resp, MAX_INDEX).await?;
    if text.trim().is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    Ok(serde_json::from_str(&text).unwrap_or_else(|_| Value::Object(Default::default())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack_registry::{add_entry, sign_entry};
    use ed25519_dalek::SigningKey;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn pubhex() -> String {
        hex::encode(key().verifying_key().to_bytes())
    }

    fn entry(url: &str) -> Entry {
        sign_entry(
            &key(),
            Entry {
                name: "notes".into(),
                version: "1.0.0".into(),
                hash: "h1".into(),
                signer_key: String::new(),
                url: url.into(),
                ports: vec![],
                grants: vec![],
                revoked: false,
                signature: String::new(),
            },
        )
    }

    fn index_text() -> String {
        add_entry(None, entry("https://example.org/notes.json")).unwrap()
    }

    async fn serve(p: &str, status: u16, body: &str) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(status).set_body_string(body))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn fetch_index_accepts_a_valid_signed_index() {
        let s = serve("/index.json", 200, &index_text()).await;
        let url = format!("{}/index.json", s.uri());
        let i = fetch_index_with(&url, &[pubhex()], true).await.unwrap();
        assert_eq!(i.entries.len(), 1);
        assert!(i.skipped.is_empty());
    }

    #[tokio::test]
    async fn fetch_index_refuses_a_non_json_document() {
        let s = serve("/index.json", 200, "<html>").await;
        let url = format!("{}/index.json", s.uri());
        assert!(fetch_index_with(&url, &[pubhex()], true).await.is_err());
    }

    #[tokio::test]
    async fn fetch_index_skips_a_tampered_entry() {
        let mut v: Value = serde_json::from_str(&index_text()).unwrap();
        v["entries"][0]["revoked"] = json!(true);
        let s = serve("/index.json", 200, &v.to_string()).await;
        let url = format!("{}/index.json", s.uri());
        let i = fetch_index_with(&url, &[pubhex()], true).await.unwrap();
        assert!(i.entries.is_empty());
        assert_eq!(i.skipped.len(), 1);
    }

    #[tokio::test]
    async fn fetch_index_refuses_http_in_production_path() {
        let s = serve("/index.json", 200, &index_text()).await;
        let url = format!("{}/index.json", s.uri());
        let e = fetch_index(&url, &[pubhex()]).await.unwrap_err();
        assert!(e.contains("https"), "{e}");
        assert!(s.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn fetch_index_error_status_is_an_error_without_the_body() {
        let s = serve("/index.json", 500, "sk-secretsecretsecret").await;
        let url = format!("{}/index.json", s.uri());
        let e = fetch_index_with(&url, &[pubhex()], true).await.unwrap_err();
        assert!(e.contains("500"), "{e}");
        assert!(!e.contains("secret"), "{e}");
    }

    #[tokio::test]
    async fn download_pack_parses_the_files_document() {
        let s = serve(
            "/p.json",
            200,
            r#"{"files":{"pack.yaml":"name: a","worker.json":"{}"}}"#,
        )
        .await;
        let e = entry(&format!("{}/p.json", s.uri()));
        let doc = download_pack_with(&e, true, MAX_PACK).await.unwrap();
        assert_eq!(doc.files.len(), 2);
        assert!(doc
            .files
            .contains(&("worker.json".to_string(), "{}".to_string())));
        assert!(parse_pack_doc(r#"{"files":{"a":1}}"#).is_err());
        assert!(parse_pack_doc(r#"{"nofiles":1}"#).is_err());
    }

    #[tokio::test]
    async fn download_pack_refuses_an_oversize_body() {
        let s = serve(
            "/p.json",
            200,
            &format!(r#"{{"files":{{"a":"{}"}}}}"#, "x".repeat(200)),
        )
        .await;
        let e = entry(&format!("{}/p.json", s.uri()));
        let r = download_pack_with(&e, true, 100).await;
        assert!(r.unwrap_err().contains("over"));
        assert!(download_pack(&e).await.is_err());
    }

    #[tokio::test]
    async fn post_json_sends_the_body_and_no_auth_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true}"#))
            .mount(&server)
            .await;
        let r = post_json(&server.uri(), &json!({"a": 1}), true)
            .await
            .unwrap();
        assert_eq!(r["ok"], json!(true));
        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 1);
        assert!(reqs[0].headers.get("authorization").is_none());
        assert!(post_json("http://x.invalid", &json!({}), false)
            .await
            .is_err());
    }
}
