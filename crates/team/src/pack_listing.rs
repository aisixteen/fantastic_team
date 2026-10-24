//! Pack listing: the packs a team can install, as messages for the web UI, the
//! socket and MCP alike. `list`, `show` and `updates` read a LOCAL index json
//! and the installed templates directory through the file bridge; nothing here
//! reaches the network and nothing here installs. Index text is untrusted: it
//! is parsed, never executed.
//!
//! The index is the signed registry format (`{"entries":[...]}`, see
//! [`pack_registry`]): an entry whose signature does not verify against the
//! listing's `trusted_keys` is skipped and reported, so a tampered `revoked`
//! flag is never honoured. The older unsigned `{"packs":[...]}` format is still
//! read for display; its entries carry no signature, so nothing decides by them
//! except that a revoked one refuses install (fail closed).

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::pack_bundle::valid_part;
use crate::pack_manifest::{self, MANIFEST_FILE};
use crate::pack_registry::{self, Decision};
use crate::puller::{KernelPeers, Peers};

/// The bundle's handler module name.
pub const HANDLER_MODULE: &str = "team_packs.tools";

/// A pack's measured record, as the index holds it. Absent fields stay `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct Measured {
    /// Tasks the record measured.
    pub tasks: Option<u64>,
    /// Share of tasks that succeeded, 0.0 to 1.0.
    pub success_rate: Option<f64>,
    /// Mean cost per task.
    pub cost_per_task: Option<f64>,
}

/// One pack version from the index: only the fields the listing shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Pack name.
    pub name: String,
    /// Pack version.
    pub version: String,
    /// One-line summary (unsigned index only; empty for signed entries).
    pub summary: String,
    /// Port names the pack exposes.
    pub ports: Vec<String>,
    /// Permissions the pack asks for.
    pub grants: Vec<String>,
    /// Number of evals, when the unsigned index has them.
    pub evals_count: Option<u64>,
    /// Measured record, when the unsigned index has one.
    pub measured: Option<Measured>,
    /// Price as the index states it; `null` when absent.
    pub price: Value,
    /// Whether the pack has been revoked. For a signed entry this is verified.
    pub revoked: bool,
    /// The verified registry entry, or `None` for an unsigned legacy entry.
    pub signed: Option<pack_registry::Entry>,
}

/// The listing's config, read from its own record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListingCfg {
    /// Local index json, read through the file bridge.
    pub index_path: String,
    /// Installed templates directory, listed through the file bridge.
    pub templates_dir: String,
    /// The file bridge agent. Defaults to `files`.
    pub store: String,
    /// Hex ed25519 public keys whose signatures the index is trusted for.
    pub trusted_keys: Vec<String>,
}

/// What [`load`] read: the listing entries, the signed index when the index is
/// one (for updates), the installed `(name, version)` pairs (version empty for
/// a bare name), and one reason per index entry that was skipped.
#[derive(Debug)]
pub struct Loaded {
    /// One entry per index entry that passed (all versions).
    pub entries: Vec<Entry>,
    /// The verified signed index, when the index is in the signed format.
    pub signed: Option<pack_registry::Index>,
    /// Installed packs as `(name, version)`.
    pub installed: Vec<(String, String)>,
    /// Index entries skipped as malformed or unverified.
    pub skipped: Vec<String>,
}

/// Parse the unsigned index json: its `packs` array. Entries without a name are skipped.
pub fn parse_index(index: &Value) -> Result<Vec<Entry>, String> {
    let packs = index["packs"]
        .as_array()
        .ok_or("index has no `packs` array")?;
    Ok(packs.iter().filter_map(parse_entry).collect())
}

fn parse_entry(v: &Value) -> Option<Entry> {
    let name = v["name"].as_str()?.to_string();
    let strings = |k: &str| -> Vec<String> {
        v[k].as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let measured = v
        .get("measured")
        .filter(|m| m.is_object())
        .map(|m| Measured {
            tasks: m["tasks"].as_u64(),
            success_rate: m["success_rate"].as_f64(),
            cost_per_task: m["cost_per_task"].as_f64(),
        });
    Some(Entry {
        name,
        version: v["version"].as_str().unwrap_or("").to_string(),
        summary: v["summary"].as_str().unwrap_or("").to_string(),
        ports: strings("ports"),
        grants: strings("grants"),
        evals_count: v["evals_count"].as_u64(),
        measured,
        price: v.get("price").cloned().unwrap_or(Value::Null),
        revoked: v["revoked"].as_bool().unwrap_or(false),
        signed: None,
    })
}

/// A verified registry entry as a listing entry. Signed entries carry no
/// summary, price, evals or measured record.
fn from_signed(e: &pack_registry::Entry) -> Entry {
    Entry {
        name: e.name.clone(),
        version: e.version.clone(),
        summary: String::new(),
        ports: e.ports.clone(),
        grants: e.grants.clone(),
        evals_count: None,
        measured: None,
        price: Value::Null,
        revoked: e.revoked,
        signed: Some(e.clone()),
    }
}

/// Split an installed directory name `<name>-<version>` at the last `-` whose
/// right side starts with a digit. A name with no such split is a bare name
/// with an empty version.
pub fn split_installed(dir: &str) -> (String, String) {
    for (i, _) in dir.rmatch_indices('-') {
        let (name, rest) = (&dir[..i], &dir[i + 1..]);
        if rest.starts_with(|c: char| c.is_ascii_digit()) && valid_part(name) && valid_part(rest) {
            return (name.to_string(), rest.to_string());
        }
    }
    (dir.to_string(), String::new())
}

/// The packs installed in the templates directory, from its listing
/// (`entries`: strings, or objects with a `name`), as `(name, version)`.
pub fn installed_packs(templates: &Value) -> Vec<(String, String)> {
    templates["entries"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().or_else(|| x["name"].as_str()))
                .map(split_installed)
                .collect()
        })
        .unwrap_or_default()
}

fn is_installed(installed: &[(String, String)], name: &str) -> bool {
    installed.iter().any(|(n, _)| n == name)
}

fn key(e: &Entry) -> Vec<u64> {
    pack_registry::version_key(&e.version)
}

/// The version a card or a show uses: the newest non-revoked one, or the
/// newest revoked one when every version is revoked.
fn newest<'a>(versions: &[&'a Entry]) -> Option<&'a Entry> {
    let live = versions
        .iter()
        .copied()
        .filter(|e| !e.revoked)
        .max_by_key(|e| key(e));
    live.or_else(|| versions.iter().copied().max_by_key(|e| key(e)))
}

/// The short card a list shows for one pack.
pub fn to_card(e: &Entry, installed: bool) -> Value {
    json!({
        "name": e.name,
        "version": e.version,
        "summary": e.summary,
        "price": e.price,
        "installed": installed,
        "revoked": e.revoked,
    })
}

/// One card per pack name for the packs matching `query` (substring of name or
/// summary, case-insensitive; none or empty matches all), sorted by name. The
/// card is the newest version (see [`newest`]).
pub fn list_view(
    entries: &[Entry],
    installed: &[(String, String)],
    query: Option<&str>,
) -> Vec<Value> {
    let q = query.map(str::to_lowercase).filter(|q| !q.is_empty());
    let mut by_name: BTreeMap<&str, Vec<&Entry>> = BTreeMap::new();
    for e in entries.iter().filter(|e| match &q {
        None => true,
        Some(q) => e.name.to_lowercase().contains(q) || e.summary.to_lowercase().contains(q),
    }) {
        by_name.entry(e.name.as_str()).or_default().push(e);
    }
    by_name
        .values()
        .filter_map(|versions| newest(versions))
        .map(|e| to_card(e, is_installed(installed, &e.name)))
        .collect()
}

fn measured_json(m: &Measured) -> Value {
    let mut o = serde_json::Map::new();
    if let Some(t) = m.tasks {
        o.insert("tasks".into(), json!(t));
    }
    if let Some(s) = m.success_rate {
        o.insert("success_rate".into(), json!(s));
    }
    if let Some(c) = m.cost_per_task {
        o.insert("cost_per_task".into(), json!(c));
    }
    Value::Object(o)
}

/// Whether a new install of this version is allowed. A signed entry is decided
/// by [`pack_registry::install_decision`]. An unsigned entry that claims to be
/// revoked is refused too: its flag is unverified, so it fails closed.
fn install_decision_json(e: &Entry) -> Value {
    let decision = match &e.signed {
        Some(reg) => pack_registry::install_decision(reg),
        None if e.revoked => Decision::Refuse("revoked (unverified index)".to_string()),
        None => Decision::Allow,
    };
    match decision {
        Decision::Allow => json!({"allow": true}),
        Decision::Refuse(reason) => json!({"allow": false, "reason": reason}),
    }
}

/// The full view of one pack, at `version` or else its newest version.
/// Unknown name or version is an error. Fields the index does not have
/// (evals, measured record) are left out, not invented.
pub fn show_view(
    entries: &[Entry],
    installed: &[(String, String)],
    name: &str,
    version: Option<&str>,
) -> Result<Value, String> {
    let same: Vec<&Entry> = entries.iter().filter(|e| e.name == name).collect();
    if same.is_empty() {
        return Err(format!("no pack named `{name}`"));
    }
    let e = match version {
        Some(v) => same
            .iter()
            .copied()
            .find(|e| e.version == v)
            .ok_or_else(|| format!("no version `{v}` of `{name}`"))?,
        None => newest(&same).ok_or_else(|| format!("no pack named `{name}`"))?,
    };
    let mut out = json!({
        "name": e.name,
        "version": e.version,
        "summary": e.summary,
        "ports": e.ports,
        "grants": e.grants,
        "price": e.price,
        "installed": is_installed(installed, name),
        "revoked": e.revoked,
        "install_decision": install_decision_json(e),
    });
    if let Some(obj) = out.as_object_mut() {
        if let Some(n) = e.evals_count {
            obj.insert("evals_count".into(), json!(n));
        }
        if let Some(m) = &e.measured {
            obj.insert("measured".into(), measured_json(m));
        }
    }
    Ok(out)
}

/// Read a whole text file through the file bridge. The bridge answers a read
/// with a page of `lines` (`{n, text}`, `eof`), not a `content` string, so the
/// pages are read in turn and joined with newlines. The last newline of the
/// file is not kept, which a JSON index does not need. A `content` reply is
/// taken as it is. An error reply is returned as the `Err`.
async fn read_text(peers: &dyn Peers, store: &str, path: &str) -> Result<String, Value> {
    let mut text = String::new();
    let mut offset = 0u64;
    loop {
        let r = peers
            .ask(
                store,
                json!({"type": "read", "path": path, "offset": offset}),
            )
            .await;
        if r.get("error").is_some() {
            return Err(r);
        }
        if let Some(c) = r.get("content").and_then(Value::as_str) {
            return Ok(c.to_string());
        }
        let Some(lines) = r.get("lines").and_then(Value::as_array) else {
            return Err(r);
        };
        for (i, l) in lines.iter().enumerate() {
            if offset > 0 || i > 0 {
                text.push('\n');
            }
            text.push_str(l.get("text").and_then(Value::as_str).unwrap_or(""));
        }
        offset += lines.len() as u64;
        let eof = r.get("eof").and_then(Value::as_bool).unwrap_or(true);
        if eof || lines.is_empty() {
            return Ok(text);
        }
    }
}

/// Read the index and the installed packs through the file bridge. A signed
/// index (`entries`) is verified against `cfg.trusted_keys`; an unsigned one
/// (`packs`) is read as is. A missing or unreadable templates listing means
/// nothing is installed.
pub async fn load(peers: &dyn Peers, cfg: &ListingCfg) -> Result<Loaded, String> {
    let text = read_text(peers, &cfg.store, &cfg.index_path)
        .await
        .map_err(|r| format!("index: {r}"))?;
    let text = text.as_str();
    let root: Value = serde_json::from_str(text).map_err(|e| format!("index is not json: {e}"))?;
    let (entries, signed, skipped): (Vec<Entry>, _, _) = if root.get("entries").is_some() {
        let index = pack_registry::Index::parse(text, &cfg.trusted_keys)?;
        let entries = index.entries.iter().map(from_signed).collect();
        let skipped = index.skipped.clone();
        (entries, Some(index), skipped)
    } else {
        (parse_index(&root)?, None, Vec::new())
    };
    let templates = peers
        .ask(
            &cfg.store,
            json!({"type": "list", "path": cfg.templates_dir}),
        )
        .await;
    Ok(Loaded {
        entries,
        signed,
        installed: installed_packs(&templates),
        skipped,
    })
}

/// The pinned hash of an installed pack, from its own `pack.yaml`.
async fn pinned_hash(
    peers: &dyn Peers,
    cfg: &ListingCfg,
    name: &str,
    version: &str,
) -> Option<String> {
    let dir = cfg.templates_dir.trim_end_matches('/');
    let r = peers
        .ask(
            &cfg.store,
            json!({"type": "read", "path": format!("{dir}/{name}-{version}/{MANIFEST_FILE}")}),
        )
        .await;
    let text = r["content"].as_str()?;
    let m = pack_manifest::Manifest::parse(text).ok()?;
    (!m.content_hash.is_empty()).then_some(m.content_hash)
}

/// The updates for installed packs, and the warnings for installed revoked ones,
/// given each installed `(name, version, pinned hash)`. Updates are opt-in: none
/// without `opt_in`. A revoked version is never offered. Installs nothing.
pub fn updates_view(
    index: &pack_registry::Index,
    installed: &[(String, String, String)],
    opt_in: bool,
) -> (Vec<Value>, Vec<String>) {
    let pins: Vec<(&str, &str)> = installed
        .iter()
        .map(|(n, _, h)| (n.as_str(), h.as_str()))
        .collect();
    let warnings = pack_registry::installed_warnings(index, &pins);
    let mut updates = Vec::new();
    for (name, version, hash) in installed {
        let Some(new) =
            pack_registry::update_candidate(index, (name.as_str(), hash.as_str()), opt_in)
        else {
            continue;
        };
        let diff = match pack_registry::resolve(index, name, hash) {
            Ok(old) => pack_registry::permission_diff(old, new),
            Err(_) => pack_registry::Diff {
                added_ports: new.ports.clone(),
                added_grants: new.grants.clone(),
                ..Default::default()
            },
        };
        updates.push(json!({
            "name": name,
            "installed_version": version,
            "new_version": new.version,
            "new_hash": new.hash,
            "permission_diff": {
                "added_ports": diff.added_ports,
                "removed_ports": diff.removed_ports,
                "added_grants": diff.added_grants,
                "removed_grants": diff.removed_grants,
            },
        }));
    }
    (updates, warnings)
}

/// The updates and warnings for what `load` read. An unsigned index has none.
pub async fn updates_for(
    peers: &dyn Peers,
    cfg: &ListingCfg,
    loaded: &Loaded,
    opt_in: bool,
) -> (Vec<Value>, Vec<String>) {
    let Some(index) = &loaded.signed else {
        return (Vec::new(), Vec::new());
    };
    let mut pinned = Vec::new();
    for (name, version) in &loaded.installed {
        if version.is_empty() {
            continue;
        }
        if let Some(hash) = pinned_hash(peers, cfg, name, version).await {
            pinned.push((name.clone(), version.clone(), hash));
        }
    }
    updates_view(index, &pinned, opt_in)
}

/// Add the `skipped` key to a reply when there is something skipped.
fn with_skipped(mut reply: Value, skipped: &[String]) -> Value {
    if !skipped.is_empty() {
        if let Some(obj) = reply.as_object_mut() {
            obj.insert("skipped".into(), json!(skipped));
        }
    }
    reply
}

/// Read the listing's config from the agent's own record.
fn cfg_of(
    kernel: &Arc<fantastic_kernel::Kernel>,
    id: &fantastic_kernel::AgentId,
) -> Result<ListingCfg, String> {
    let meta = kernel
        .agents
        .get(id)
        .map(|e| e.meta.read().expect("meta").clone())
        .unwrap_or_default();
    let get = |k: &str| {
        meta.get(k)
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|v| !v.trim().is_empty())
    };
    let bad_keys = || "team_packs: `trusted_keys` must be a list of hex strings".to_string();
    let trusted_keys = match meta.get("trusted_keys") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|k| k.as_str().map(str::to_string))
            .collect::<Option<Vec<String>>>()
            .ok_or_else(bad_keys)?,
        Some(_) => return Err(bad_keys()),
    };
    Ok(ListingCfg {
        index_path: get("index_path").ok_or("team_packs: record needs `index_path`")?,
        templates_dir: get("templates_dir").ok_or("team_packs: record needs `templates_dir`")?,
        store: get("store").unwrap_or_else(|| "files".to_string()),
        trusted_keys,
    })
}

/// The pack listing bundle: `list`, `show` and `updates`.
pub struct ListingBundle;

#[async_trait::async_trait]
impl fantastic_kernel::bundle::Bundle for ListingBundle {
    fn name(&self) -> &str {
        HANDLER_MODULE
    }

    fn verb_docs(&self) -> Value {
        json!({
            "list": "list {query?} → {packs: [{name, version, summary, price, installed, revoked}], skipped?}",
            "show": "show {name, version?} → {name, version, summary, ports, grants, evals_count?, measured?, price, installed, revoked, install_decision: {allow, reason?}, skipped?}",
            "updates": "updates {opt_in?} → {updates: [{name, installed_version, new_version, new_hash, permission_diff: {added_ports, removed_ports, added_grants, removed_grants}}], warnings: [...], skipped?} (opt_in defaults false: no updates without it; installs nothing)",
        })
    }

    async fn handle(
        &self,
        agent_id: &fantastic_kernel::AgentId,
        payload: &Value,
        kernel: &Arc<fantastic_kernel::Kernel>,
    ) -> Result<fantastic_kernel::bundle::Reply, fantastic_kernel::bundle::BundleError> {
        let t = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if t == "boot" {
            return Ok(None);
        }
        let peers = KernelPeers(kernel.clone());
        let out = match cfg_of(kernel, agent_id) {
            Err(e) => json!({"error": e}),
            Ok(cfg) => match t {
                "list" | "show" | "updates" => match load(&peers, &cfg).await {
                    Err(e) => json!({"error": e}),
                    Ok(loaded) => match t {
                        "list" => {
                            let query = payload.get("query").and_then(|v| v.as_str());
                            let cards = list_view(&loaded.entries, &loaded.installed, query);
                            with_skipped(json!({"packs": cards}), &loaded.skipped)
                        }
                        "show" => {
                            let name = payload.get("name").and_then(|v| v.as_str()).unwrap_or("");
                            let version = payload.get("version").and_then(|v| v.as_str());
                            match show_view(&loaded.entries, &loaded.installed, name, version) {
                                Ok(v) => with_skipped(v, &loaded.skipped),
                                Err(e) => json!({"error": e}),
                            }
                        }
                        _ => {
                            let opt_in = payload
                                .get("opt_in")
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                            let (updates, warnings) =
                                updates_for(&peers, &cfg, &loaded, opt_in).await;
                            with_skipped(
                                json!({"updates": updates, "warnings": warnings}),
                                &loaded.skipped,
                            )
                        }
                    },
                },
                other => json!({"error": format!("team_packs: unknown type '{other}'")}),
            },
        };
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    /// The index path the fake serves.
    const INDEX_PATH: &str = "packs/index.json";

    struct Fake {
        index: Option<String>,
        installed: Vec<String>,
        files: BTreeMap<String, String>,
    }

    #[async_trait::async_trait]
    impl Peers for Fake {
        async fn ask(&self, _to: &str, p: Value) -> Value {
            match p["type"].as_str() {
                Some("read") => {
                    let path = p["path"].as_str().unwrap_or("");
                    let text = if path == INDEX_PATH {
                        self.index.clone()
                    } else {
                        self.files.get(path).cloned()
                    };
                    match text {
                        Some(t) => json!({"content": t}),
                        None => json!({"error": "not found"}),
                    }
                }
                Some("list") => json!({"entries": self.installed}),
                _ => json!({"error": "unknown"}),
            }
        }
    }

    const INDEX: &str = r#"{"packs":[
        {"name":"zeta","version":"1.0","summary":"Zeta pack","ports":["in"],"grants":["read"],"evals_count":3,"measured":{"tasks":10,"success_rate":0.9,"cost_per_task":0.02},"price":0},
        {"name":"alpha","version":"0.1","summary":"Alpha search tool","ports":[],"grants":[],"price":5},
        {"name":"mid","version":"2.0","summary":"Mid","ports":["out"],"grants":[],"revoked":true,"price":1}
    ]}"#;

    fn cfg_with(trusted_keys: Vec<String>) -> ListingCfg {
        ListingCfg {
            index_path: INDEX_PATH.into(),
            templates_dir: "templates".into(),
            store: "files".into(),
            trusted_keys,
        }
    }

    fn fake(index: Option<&str>, installed: &[&str], files: &[(&str, &str)]) -> Fake {
        Fake {
            index: index.map(str::to_string),
            installed: installed.iter().map(|s| s.to_string()).collect(),
            files: files
                .iter()
                .map(|(p, t)| (p.to_string(), t.to_string()))
                .collect(),
        }
    }

    async fn loaded(installed: &[&str]) -> Loaded {
        load(&fake(Some(INDEX), installed, &[]), &cfg_with(vec![]))
            .await
            .unwrap()
    }

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn pubhex(k: &SigningKey) -> String {
        hex::encode(k.verifying_key().to_bytes())
    }

    /// One signed registry entry as the index holds it, signed by `k`.
    fn signed_entry(
        k: &SigningKey,
        name: &str,
        version: &str,
        hash: &str,
        ports: &[&str],
        revoked: bool,
    ) -> Value {
        let e = pack_registry::Entry {
            name: name.into(),
            version: version.into(),
            hash: hash.into(),
            signer_key: pubhex(k),
            url: format!("https://example.org/{name}-{version}.tar"),
            ports: ports.iter().map(|p| p.to_string()).collect(),
            grants: vec![],
            revoked,
            signature: String::new(),
        };
        let sig = k.sign(&pack_registry::signing_payload(&e));
        json!({
            "name": e.name,
            "version": e.version,
            "hash": e.hash,
            "signer_key": e.signer_key,
            "url": e.url,
            "ports": e.ports,
            "grants": e.grants,
            "revoked": e.revoked,
            "signature": hex::encode(sig.to_bytes()),
        })
    }

    fn signed_index(entries: Vec<Value>) -> String {
        json!({ "entries": entries }).to_string()
    }

    /// A pack.yaml for `name` at `version` with the given pinned hash.
    fn pack_yaml(name: &str, version: &str, hash: &str) -> String {
        format!("name: {name}\nversion: {version}\nauthor: a\nlicense: MIT\ncontent_hash: {hash}\n")
    }

    #[tokio::test]
    async fn list_marks_installed_packs() {
        let l = loaded(&["alpha"]).await;
        let cards = list_view(&l.entries, &l.installed, None);
        let alpha = cards.iter().find(|c| c["name"] == "alpha").unwrap();
        let zeta = cards.iter().find(|c| c["name"] == "zeta").unwrap();
        assert_eq!(alpha["installed"], json!(true));
        assert_eq!(zeta["installed"], json!(false));
    }

    #[tokio::test]
    async fn list_marks_installed_for_name_version_dirs() {
        let l = loaded(&["alpha-0.1"]).await;
        let cards = list_view(&l.entries, &l.installed, None);
        let alpha = cards.iter().find(|c| c["name"] == "alpha").unwrap();
        assert_eq!(alpha["installed"], json!(true));
    }

    #[test]
    fn installed_dirs_split_on_the_version() {
        assert_eq!(
            split_installed("pack-2-1.0.0"),
            ("pack-2".to_string(), "1.0.0".to_string())
        );
        assert_eq!(
            split_installed("my-tool"),
            ("my-tool".to_string(), String::new())
        );
    }

    #[tokio::test]
    async fn list_filters_by_query() {
        let l = loaded(&[]).await;
        let by_name = list_view(&l.entries, &l.installed, Some("ALPHA"));
        assert_eq!(by_name.len(), 1);
        assert_eq!(by_name[0]["name"], json!("alpha"));
        let by_summary = list_view(&l.entries, &l.installed, Some("search"));
        assert_eq!(by_summary.len(), 1);
        assert_eq!(list_view(&l.entries, &l.installed, Some("")).len(), 3);
    }

    #[tokio::test]
    async fn list_is_sorted_by_name() {
        let l = loaded(&[]).await;
        let names: Vec<Value> = list_view(&l.entries, &l.installed, None)
            .iter()
            .map(|c| c["name"].clone())
            .collect();
        assert_eq!(names, vec![json!("alpha"), json!("mid"), json!("zeta")]);
    }

    #[tokio::test]
    async fn show_returns_ports_grants_and_evals_count() {
        let l = loaded(&[]).await;
        let v = show_view(&l.entries, &l.installed, "zeta", None).unwrap();
        assert_eq!(v["ports"], json!(["in"]));
        assert_eq!(v["grants"], json!(["read"]));
        assert_eq!(v["evals_count"], json!(3));
        assert_eq!(v["measured"]["tasks"], json!(10));
    }

    #[tokio::test]
    async fn show_omits_a_missing_measured_record() {
        let l = loaded(&[]).await;
        let v = show_view(&l.entries, &l.installed, "alpha", None).unwrap();
        assert!(v.get("measured").is_none());
        assert!(v.get("evals_count").is_none());
    }

    #[tokio::test]
    async fn show_unknown_name_is_an_error() {
        let l = loaded(&[]).await;
        assert!(show_view(&l.entries, &l.installed, "nope", None).is_err());
    }

    #[tokio::test]
    async fn a_revoked_pack_is_marked() {
        let l = loaded(&[]).await;
        let cards = list_view(&l.entries, &l.installed, None);
        let mid = cards.iter().find(|c| c["name"] == "mid").unwrap();
        assert_eq!(mid["revoked"], json!(true));
    }

    #[tokio::test]
    async fn list_from_signed_index_ignores_tampered_revoked() {
        let (k1, k3) = (key(1), key(3));
        let mut tampered = signed_entry(&k1, "notes", "1.1.0", "h2", &["read"], false);
        // Signed as not revoked, then the flag is flipped without re-signing.
        tampered["revoked"] = json!(true);
        let untrusted = signed_entry(&k3, "mid", "1.0.0", "h5", &[], false);
        let index = signed_index(vec![
            signed_entry(&k1, "notes", "1.0.0", "h1", &["read"], false),
            tampered,
            untrusted,
            signed_entry(&k1, "zeta", "1.0.0", "h9", &[], false),
        ]);
        let f = fake(Some(&index), &[], &[]);
        let l = load(&f, &cfg_with(vec![pubhex(&k1)])).await.unwrap();
        assert_eq!(l.skipped.len(), 2, "{:?}", l.skipped);
        assert!(
            l.skipped.iter().any(|s| s.contains("bad signature")),
            "{:?}",
            l.skipped
        );
        assert!(
            l.skipped.iter().any(|s| s.contains("untrusted")),
            "{:?}",
            l.skipped
        );
        let cards = list_view(&l.entries, &l.installed, None);
        let names: Vec<&str> = cards.iter().map(|c| c["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["notes", "zeta"]);
        let notes = cards.iter().find(|c| c["name"] == "notes").unwrap();
        assert_eq!(notes["version"], json!("1.0.0"));
        assert_eq!(notes["revoked"], json!(false));
    }

    #[tokio::test]
    async fn list_shows_newest_unrevoked_version_per_pack() {
        let k1 = key(1);
        let index = signed_index(vec![
            signed_entry(&k1, "notes", "1.0.0", "h1", &[], false),
            signed_entry(&k1, "notes", "1.2.0", "h2", &[], false),
            signed_entry(&k1, "notes", "1.3.0", "h3", &[], true),
            signed_entry(&k1, "tool", "1.9.0", "t1", &[], false),
            signed_entry(&k1, "tool", "1.10.0", "t2", &[], false),
        ]);
        let f = fake(Some(&index), &[], &[]);
        let l = load(&f, &cfg_with(vec![pubhex(&k1)])).await.unwrap();
        let cards = list_view(&l.entries, &l.installed, None);
        assert_eq!(cards.len(), 2, "{cards:?}");
        let notes = cards.iter().find(|c| c["name"] == "notes").unwrap();
        assert_eq!(notes["version"], json!("1.2.0"));
        let tool = cards.iter().find(|c| c["name"] == "tool").unwrap();
        assert_eq!(tool["version"], json!("1.10.0"));
    }

    #[tokio::test]
    async fn updates_requires_opt_in_and_shows_permission_diff() {
        let (k1, k2) = (key(1), key(2));
        let index = signed_index(vec![
            signed_entry(&k1, "notes", "1.0.0", "h1", &["read"], false),
            signed_entry(&k1, "notes", "1.2.0", "h2", &["read", "write"], false),
            signed_entry(&k2, "notes", "1.3.0", "h4", &[], true),
        ]);
        // Installed: 1.0.0 (pinned h1) and 1.3.0 (pinned h4, now revoked).
        let f = fake(
            Some(&index),
            &["notes-1.0.0", "notes-1.3.0"],
            &[
                (
                    "templates/notes-1.0.0/pack.yaml",
                    &pack_yaml("notes", "1.0.0", "h1"),
                ),
                (
                    "templates/notes-1.3.0/pack.yaml",
                    &pack_yaml("notes", "1.3.0", "h4"),
                ),
            ],
        );
        let cfg = cfg_with(vec![pubhex(&k1), pubhex(&k2)]);
        let l = load(&f, &cfg).await.unwrap();

        let (no_opt, warnings) = updates_for(&f, &cfg, &l, false).await;
        assert!(no_opt.is_empty());
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("revoked"));

        let (offered, _) = updates_for(&f, &cfg, &l, true).await;
        assert_eq!(offered.len(), 2, "{offered:?}");
        let from_1_0 = offered
            .iter()
            .find(|u| u["installed_version"] == json!("1.0.0"))
            .unwrap();
        assert_eq!(from_1_0["new_version"], json!("1.2.0"));
        assert_eq!(from_1_0["new_hash"], json!("h2"));
        assert_eq!(from_1_0["permission_diff"]["added_ports"], json!(["write"]));
        assert!(offered.iter().all(|u| u["new_version"] != json!("1.3.0")));
    }

    #[tokio::test]
    async fn show_refuses_install_for_revoked() {
        let k1 = key(1);
        let index = signed_index(vec![
            signed_entry(&k1, "notes", "1.0.0", "h1", &[], false),
            signed_entry(&k1, "notes", "1.3.0", "h4", &[], true),
        ]);
        let f = fake(Some(&index), &[], &[]);
        let l = load(&f, &cfg_with(vec![pubhex(&k1)])).await.unwrap();
        let v = show_view(&l.entries, &l.installed, "notes", Some("1.3.0")).unwrap();
        assert_eq!(v["revoked"], json!(true));
        assert_eq!(
            v["install_decision"],
            json!({"allow": false, "reason": "revoked"})
        );
        let ok = show_view(&l.entries, &l.installed, "notes", Some("1.0.0")).unwrap();
        assert_eq!(ok["install_decision"], json!({"allow": true}));
    }

    #[test]
    fn register_team_registers_listing() {
        let mut reg = fantastic_kernel::BundleRegistry::new();
        crate::register_team(&mut reg);
        assert!(reg.get(HANDLER_MODULE).is_some());
    }
}
