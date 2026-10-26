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

use crate::pack_bundle::{refused_by_scan, valid_part};
use crate::pack_hosted;
use crate::pack_manifest::{self, MANIFEST_FILE};
use crate::pack_record::{publishable_verified, record_from_json, verify_record, MeasuredRecord};
use crate::pack_registry::{self, Decision};
use crate::pack_scan;
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
    /// The hosted registry: its https url serves the signed index (GET),
    /// takes submissions (POST), `<url>/report` takes reports and takes the
    /// re-signed index of a takedown. `None` means local only: nothing here
    /// reaches the network.
    pub registry_url: Option<String>,
    /// Allow an http registry url. Never read from the record, so it is only
    /// true in tests.
    pub allow_insecure: bool,
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
/// summary, price or evals; `measured` is the entry's measured record when it
/// verified (see [`verified_measured`]).
fn from_signed(e: &pack_registry::Entry, measured: Option<Measured>) -> Entry {
    Entry {
        name: e.name.clone(),
        version: e.version.clone(),
        summary: String::new(),
        ports: e.ports.clone(),
        grants: e.grants.clone(),
        evals_count: None,
        measured,
        price: Value::Null,
        revoked: e.revoked,
        signed: Some(e.clone()),
    }
}

/// The measured record beside a verified signed entry in the raw index, only
/// when it is a signed record for this entry's hash whose signature verifies.
/// Any other `measured` value is dropped.
fn verified_measured(raw_entries: &[Value], e: &pack_registry::Entry) -> Option<Measured> {
    let raw = raw_entries
        .iter()
        .find(|r| r.get("signature").and_then(Value::as_str) == Some(e.signature.as_str()))?;
    let rec = record_from_json(raw.get("measured")?)?;
    if rec.pack_hash != e.hash || verify_record(&rec).is_err() {
        return None;
    }
    Some(Measured {
        tasks: Some(rec.tasks),
        success_rate: Some(rec.success_rate),
        cost_per_task: rec.cost_per_task,
    })
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

/// The packs installed in the templates directory, from its listing, as
/// `(name, version)`. The file bridge answers `files: [{name, type}]` (only
/// folders count: a pack installs as a folder); a plain `entries` list of
/// strings, or of objects with a `name`, is read too.
pub fn installed_packs(templates: &Value) -> Vec<(String, String)> {
    let named = |a: &Vec<Value>, dirs_only: bool| -> Vec<(String, String)> {
        a.iter()
            .filter(|x| !dirs_only || x["type"].as_str().is_none_or(|t| t == "dir"))
            .filter_map(|x| x.as_str().or_else(|| x["name"].as_str()))
            .map(split_installed)
            .collect()
    };
    if let Some(a) = templates["files"].as_array() {
        return named(a, true);
    }
    templates["entries"]
        .as_array()
        .map(|a| named(a, false))
        .unwrap_or_default()
}

pub(crate) fn is_installed(installed: &[(String, String)], name: &str) -> bool {
    installed.iter().any(|(n, _)| n == name)
}

fn key(e: &Entry) -> Vec<u64> {
    pack_registry::version_key(&e.version)
}

/// The version a card or a show uses: the newest non-revoked one, or the
/// newest revoked one when every version is revoked.
pub(crate) fn newest<'a>(versions: &[&'a Entry]) -> Option<&'a Entry> {
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

/// Read the index and the installed packs. The index comes from the hosted
/// registry when `cfg.registry_url` is set, else from the local file through
/// the file bridge. A signed index (`entries`) is verified against
/// `cfg.trusted_keys`; an unsigned one (`packs`) is read as is. A missing or
/// unreadable templates listing means nothing is installed.
pub async fn load(peers: &dyn Peers, cfg: &ListingCfg) -> Result<Loaded, String> {
    let text = match &cfg.registry_url {
        Some(url) => pack_hosted::fetch_index_text(url, cfg.allow_insecure)
            .await
            .map_err(|e| format!("index: {e}"))?,
        None => read_text(peers, &cfg.store, &cfg.index_path)
            .await
            .map_err(|r| format!("index: {r}"))?,
    };
    let templates = peers
        .ask(
            &cfg.store,
            json!({"type": "list", "path": cfg.templates_dir}),
        )
        .await;
    parse_loaded(&text, cfg, installed_packs(&templates))
}

/// Parse index text into what [`load`] returns, given the installed packs.
/// Pure: no bridge, no network.
pub fn parse_loaded(
    text: &str,
    cfg: &ListingCfg,
    installed: Vec<(String, String)>,
) -> Result<Loaded, String> {
    let root: Value = serde_json::from_str(text).map_err(|e| format!("index is not json: {e}"))?;
    let (entries, signed, skipped): (Vec<Entry>, _, _) = if root.get("entries").is_some() {
        let index = pack_registry::Index::parse(text, &cfg.trusted_keys)?;
        let raw = root["entries"].as_array().cloned().unwrap_or_default();
        let entries = index
            .entries
            .iter()
            .map(|e| from_signed(e, verified_measured(&raw, e)))
            .collect();
        let skipped = index.skipped.clone();
        (entries, Some(index), skipped)
    } else {
        (parse_index(&root)?, None, Vec::new())
    };
    Ok(Loaded {
        entries,
        signed,
        installed,
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
    let path = format!("{dir}/{name}-{version}/{MANIFEST_FILE}");
    let text = read_text(peers, &cfg.store, &path).await.ok()?;
    let m = pack_manifest::Manifest::parse(&text).ok()?;
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
    let pinned = pins_for(peers, cfg, loaded).await;
    updates_view(index, &pinned, opt_in)
}

/// Each installed `(name, version, pinned hash)` whose own `pack.yaml` pins one.
pub async fn pins_for(
    peers: &dyn Peers,
    cfg: &ListingCfg,
    loaded: &Loaded,
) -> Vec<(String, String, String)> {
    let mut pinned = Vec::new();
    for (name, version) in &loaded.installed {
        if version.is_empty() {
            continue;
        }
        if let Some(hash) = pinned_hash(peers, cfg, name, version).await {
            pinned.push((name.clone(), version.clone(), hash));
        }
    }
    pinned
}

/// Whether a bridge error reply says the file does not exist (a new index),
/// as opposed to any other read failure, which must not start a new index.
fn is_missing(reply: &Value) -> bool {
    let e = reply
        .get("error")
        .map(|e| e.to_string().to_lowercase())
        .unwrap_or_default();
    ["not found", "no such file", "does not exist"]
        .iter()
        .any(|m| e.contains(m))
}

/// The arguments `publish` and `submit` share: `pack`, `url`, `signing_key`
/// and the optional `grants`.
fn publish_args(
    payload: &Value,
    verb: &str,
) -> Result<(String, String, String, Vec<String>), Value> {
    let err = |m: &str| json!({"error": m});
    let text_of = |k: &str| payload.get(k).and_then(Value::as_str).map(str::trim);
    let (Some(pack), Some(url), Some(key_path)) =
        (text_of("pack"), text_of("url"), text_of("signing_key"))
    else {
        return Err(err(&format!(
            "{verb} needs `pack`, `url` and `signing_key`"
        )));
    };
    if pack.is_empty() || url.is_empty() || key_path.is_empty() {
        return Err(err(&format!(
            "{verb} needs `pack`, `url` and `signing_key`"
        )));
    }
    let grants: Vec<String> = match payload.get("grants") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => {
            match a
                .iter()
                .map(|g| g.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()
            {
                Some(g) => g,
                None => return Err(err("`grants` must be a list of strings")),
            }
        }
        Some(_) => return Err(err("`grants` must be a list of strings")),
    };
    Ok((
        pack.to_string(),
        url.to_string(),
        key_path.to_string(),
        grants,
    ))
}

/// A pack folder that passed the manifest and content-hash checks, with its
/// scan report (not yet acted on).
struct Reviewed {
    manifest: pack_manifest::Manifest,
    manifest_text: String,
    bytes: Vec<(String, Vec<u8>)>,
    report: pack_scan::ScanReport,
}

/// Read a pack folder through the bridge and check it like an install: the
/// manifest, safe name and version, every listed file, the content hash. The
/// scan runs and its report is returned; the caller decides what a finding
/// means. An `Err` is the reply to send.
async fn review_pack(peers: &dyn Peers, cfg: &ListingCfg, pack: &str) -> Result<Reviewed, Value> {
    let err = |m: &str| json!({"error": m});
    let manifest_path = format!("{pack}/{MANIFEST_FILE}");
    let Ok(manifest_text) = read_text(peers, &cfg.store, &manifest_path).await else {
        return Err(err("pack has no pack.yaml"));
    };
    let manifest = match pack_manifest::Manifest::parse(&manifest_text) {
        Ok(m) => m,
        Err(e) => return Err(err(&format!("pack.yaml: {e}"))),
    };
    if let Err(e) = manifest.validate() {
        return Err(err(&e));
    }
    if !valid_part(&manifest.name) || !valid_part(&manifest.version) {
        return Err(err("pack name or version is not a safe path segment"));
    }
    let mut bytes: Vec<(String, Vec<u8>)> = Vec::new();
    for rel in &manifest.files {
        match read_text(peers, &cfg.store, &format!("{pack}/{rel}")).await {
            Ok(t) => bytes.push((rel.clone(), t.into_bytes())),
            Err(_) => return Err(err(&format!("pack file missing: {rel}"))),
        }
    }
    if pack_manifest::content_hash(&bytes) != manifest.content_hash {
        return Err(err("content hash mismatch"));
    }
    let report = pack_scan::scan(&bytes);
    Ok(Reviewed {
        manifest,
        manifest_text,
        bytes,
        report,
    })
}

/// Read and parse the signing key file. The error is fixed text.
async fn read_key(
    peers: &dyn Peers,
    cfg: &ListingCfg,
    key_path: &str,
) -> Result<ed25519_dalek::SigningKey, Value> {
    match read_text(peers, &cfg.store, key_path).await {
        Ok(t) => pack_registry::signing_key_from_hex(&t).map_err(|e| json!({"error": e})),
        Err(_) => Err(json!({"error": "signing key file could not be read"})),
    }
}

/// The signed entry for a reviewed pack.
fn signed_entry_of(
    key: &ed25519_dalek::SigningKey,
    manifest: &pack_manifest::Manifest,
    url: &str,
    grants: Vec<String>,
) -> pack_registry::Entry {
    pack_registry::sign_entry(
        key,
        pack_registry::Entry {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            hash: manifest.content_hash.clone(),
            signer_key: String::new(),
            url: url.to_string(),
            ports: manifest.ports.clone(),
            grants,
            revoked: false,
            signature: String::new(),
        },
    )
}

/// Publish a packed pack to the LOCAL index: check it, sign it with the key in
/// the file at `signing_key`, and append the signed entry to `cfg.index_path`.
/// Payload: `{pack, url, signing_key, grants?}`. The pack must pass the same
/// checks as an install (manifest, content hash, secrets scan) before anything
/// is signed or written. Grants come only from the payload, since `pack.yaml`
/// has none. The key and its seed never appear in a reply. The signer need not
/// be trusted here (trust is the reader's decision); the reply says
/// `trusted: false` when `list` will skip the entry.
pub async fn publish(peers: &dyn Peers, cfg: &ListingCfg, payload: &Value) -> Value {
    let err = |m: &str| json!({"error": m});
    let (pack, url, key_path, grants) = match publish_args(payload, "publish") {
        Ok(a) => a,
        Err(e) => return e,
    };
    let (pack, url, key_path) = (pack.as_str(), url.as_str(), key_path.as_str());
    let Reviewed {
        manifest, report, ..
    } = match review_pack(peers, cfg, pack).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    if report.refused {
        return refused_by_scan(&report);
    }
    let key = match read_key(peers, cfg, key_path).await {
        Ok(k) => k,
        Err(e) => return e,
    };
    let entry = signed_entry_of(&key, &manifest, url, grants);
    let current = match read_text(peers, &cfg.store, &cfg.index_path).await {
        Ok(t) => Some(t),
        Err(r) if is_missing(&r) => None,
        Err(_) => return err("index could not be read"),
    };
    let signer_key = entry.signer_key.clone();
    let hash = entry.hash.clone();
    let (measured, measured_reason) = checked_measured(payload, &hash);
    let updated = match pack_registry::add_entry(current.as_deref(), entry) {
        Ok(t) => match &measured {
            Some(rec) => match attach_measured(&t, rec) {
                Some(t) => t,
                None => return err("index could not be updated"),
            },
            None => t,
        },
        Err(e) => return err(&e),
    };
    let w = peers
        .ask(
            &cfg.store,
            json!({"type": "write", "path": cfg.index_path, "bytes": updated}),
        )
        .await;
    if w.get("error").is_some() {
        return err("index could not be written");
    }
    let trusted = cfg.trusted_keys.contains(&signer_key);
    let mut reply = json!({"published": {
        "name": manifest.name,
        "version": manifest.version,
        "hash": hash,
        "signer_key": signer_key,
    }, "trusted": trusted});
    if !trusted {
        reply["note"] = json!(
            "the signer is not in this listing's trusted_keys, so `list` skips the entry until it is"
        );
    }
    reply["measured_published"] = json!(measured.is_some());
    if measured.is_none() {
        reply["measured_reason"] = json!(measured_reason);
    }
    reply
}

/// A required, trimmed, non-empty string from the payload.
fn req<'a>(payload: &'a Value, k: &str) -> Option<&'a str> {
    payload
        .get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

/// Download a pack from the signed index into a staging folder, ready for
/// `install`. Payload: `{name, hash, stage, version?}`. The entry is found by
/// name and pinned hash in the SIGNED index; a revoked entry is refused; the
/// downloaded files must match the pin and pass the scan before anything is
/// written (`pack_bundle::stage_download`). It installs nothing.
pub async fn fetch<P: Peers>(peers: &P, cfg: &ListingCfg, payload: &Value) -> Value {
    let allow_insecure = cfg.allow_insecure;
    fetch_via(peers, cfg, payload, |e| async move {
        pack_hosted::download_pack_with(&e, allow_insecure, pack_hosted::MAX_PACK).await
    })
    .await
}

/// [`fetch`] with the download step given, so a test can serve the pack from a
/// mock whose url is not https.
async fn fetch_via<P, F, Fut>(peers: &P, cfg: &ListingCfg, payload: &Value, download: F) -> Value
where
    P: Peers,
    F: FnOnce(pack_registry::Entry) -> Fut,
    Fut: std::future::Future<Output = Result<pack_hosted::PackDoc, String>>,
{
    let err = |m: &str| json!({"error": m});
    let (Some(name), Some(hash), Some(stage)) = (
        req(payload, "name"),
        req(payload, "hash"),
        req(payload, "stage"),
    ) else {
        return err("fetch needs `name`, `hash` and `stage`");
    };
    let loaded = match load(peers, cfg).await {
        Ok(l) => l,
        Err(e) => return err(&e),
    };
    let Some(index) = &loaded.signed else {
        return err("fetch needs a signed index");
    };
    let entry = match pack_registry::resolve(index, name, hash) {
        Ok(e) => e,
        Err(e) => return err(&e),
    };
    if let Some(v) = req(payload, "version") {
        if v != entry.version {
            return err(&format!(
                "`{name}` at that hash is version {}, not {v}",
                entry.version
            ));
        }
    }
    if let Decision::Refuse(reason) = pack_registry::install_decision(entry) {
        return err(&format!("install refused: {reason}"));
    }
    let doc = match download(entry.clone()).await {
        Ok(d) => d,
        Err(e) => return err(&e),
    };
    let mut staged =
        crate::pack_bundle::stage_download(peers, &doc.files, &entry.hash, stage).await;
    if staged.get("error").is_none() {
        staged["next"] = json!(format!(
            "packs install --pack {stage} --into <templates dir>"
        ));
    }
    staged
}

/// Submit a packed pack to the hosted registry. Payload: `{pack, url,
/// signing_key, grants?}`. The pack goes through the same review as a publish,
/// but a finding of any kind HOLDS it instead of listing it: the reply is
/// `{listed: false, status: "held", findings: [{path, rule}]}` and nothing is
/// sent. A clean pack is signed and POSTed with its files to `registry_url`.
pub async fn submit(peers: &dyn Peers, cfg: &ListingCfg, payload: &Value) -> Value {
    let err = |m: &str| json!({"error": m});
    let Some(registry) = &cfg.registry_url else {
        return err("no registry_url");
    };
    let (pack, url, key_path, grants) = match publish_args(payload, "submit") {
        Ok(a) => a,
        Err(e) => return e,
    };
    let reviewed = match review_pack(peers, cfg, &pack).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    if !reviewed.report.findings.is_empty() {
        let findings: Vec<Value> = reviewed
            .report
            .findings
            .iter()
            .map(|f| json!({"path": f.path, "rule": f.rule}))
            .collect();
        return json!({"listed": false, "status": "held", "findings": findings});
    }
    let key = match read_key(peers, cfg, &key_path).await {
        Ok(k) => k,
        Err(e) => return e,
    };
    let entry = signed_entry_of(&key, &reviewed.manifest, &url, grants);
    if let Err(e) = pack_registry::verify_entry(&entry, std::slice::from_ref(&entry.signer_key)) {
        return err(&e);
    }
    let mut files = serde_json::Map::new();
    files.insert(MANIFEST_FILE.to_string(), json!(reviewed.manifest_text));
    for (rel, b) in &reviewed.bytes {
        files.insert(rel.clone(), json!(String::from_utf8_lossy(b)));
    }
    let body = json!({"entry": pack_registry::entry_json(&entry), "files": files});
    match pack_hosted::post_json(registry, &body, cfg.allow_insecure).await {
        Ok(reply) => json!({
            "listed": true,
            "name": entry.name,
            "version": entry.version,
            "hash": entry.hash,
            "signer_key": entry.signer_key,
            "registry": reply,
        }),
        Err(e) => err(&format!("submit failed: {e}")),
    }
}

/// Report a pack to the registry. Payload: `{name, version, reason}`; the
/// reason is non-empty and at most 2000 characters. POSTs to
/// `<registry_url>/report`.
pub async fn report(cfg: &ListingCfg, payload: &Value) -> Value {
    let err = |m: &str| json!({"error": m});
    let Some(registry) = &cfg.registry_url else {
        return err("no registry_url");
    };
    let (Some(name), Some(version), Some(reason)) = (
        req(payload, "name"),
        req(payload, "version"),
        req(payload, "reason"),
    ) else {
        return err("report needs `name`, `version` and `reason`");
    };
    if !valid_part(name) || !valid_part(version) {
        return err("name or version is not a safe path segment");
    }
    if reason.chars().count() > 2000 {
        return err("reason is over 2000 characters");
    }
    let url = format!("{}/report", registry.trim_end_matches('/'));
    let body = json!({"name": name, "version": version, "reason": reason});
    match pack_hosted::post_json(&url, &body, cfg.allow_insecure).await {
        Ok(_) => json!({"reported": true}),
        Err(e) => err(&format!("report failed: {e}")),
    }
}

/// Take a version down: revoke it in the LOCAL index (re-signed with the key
/// in `signing_key`, so the flag is covered by the signature) and, when
/// `registry_url` is set, POST the updated index there as
/// `{takedown: {name, version}, index}` so the revocation reaches the hosted
/// index. Payload: `{name, version, signing_key}`. The key never appears in a
/// reply.
pub async fn takedown(peers: &dyn Peers, cfg: &ListingCfg, payload: &Value) -> Value {
    let err = |m: &str| json!({"error": m});
    let (Some(name), Some(version), Some(key_path)) = (
        req(payload, "name"),
        req(payload, "version"),
        req(payload, "signing_key"),
    ) else {
        return err("takedown needs `name`, `version` and `signing_key`");
    };
    if !valid_part(name) || !valid_part(version) {
        return err("name or version is not a safe path segment");
    }
    let key = match read_key(peers, cfg, key_path).await {
        Ok(k) => k,
        Err(e) => return e,
    };
    let Ok(current) = read_text(peers, &cfg.store, &cfg.index_path).await else {
        return err("index could not be read");
    };
    let updated = match pack_registry::revoke_entry(&current, name, version, &key) {
        Ok(t) => t,
        Err(e) => return err(&e),
    };
    let w = peers
        .ask(
            &cfg.store,
            json!({"type": "write", "path": cfg.index_path, "bytes": updated}),
        )
        .await;
    if w.get("error").is_some() {
        return err("index could not be written");
    }
    let mut reply = json!({"revoked": {"name": name, "version": version}, "pushed": false});
    if let Some(registry) = &cfg.registry_url {
        let index: Value = serde_json::from_str(&updated).unwrap_or(Value::Null);
        let body = json!({"takedown": {"name": name, "version": version}, "index": index});
        match pack_hosted::post_json(registry, &body, cfg.allow_insecure).await {
            Ok(_) => reply["pushed"] = json!(true),
            Err(e) => {
                reply["error"] = json!(format!(
                    "revoked locally, but the registry push failed: {e}"
                ));
            }
        }
    }
    reply
}

/// The payload's measured record if it may be published with the entry for
/// `hash`: the owner opted in, the record parses, its signature verifies and it
/// is for this pack. Otherwise None and the reason.
fn checked_measured(payload: &Value, hash: &str) -> (Option<MeasuredRecord>, &'static str) {
    let Some(m) = payload.get("measured").filter(|m| !m.is_null()) else {
        return (None, "no measured record given");
    };
    if payload.get("opt_in").and_then(Value::as_bool) != Some(true) {
        return (None, "opt_in is not set");
    }
    let Some(rec) = record_from_json(m) else {
        return (None, "measured record is malformed");
    };
    let Some(rec) = publishable_verified(&rec, true) else {
        return (
            None,
            "measured record is unsigned or its signature does not verify",
        );
    };
    if rec.pack_hash != hash {
        return (None, "measured record is for another pack");
    }
    (Some(rec), "")
}

/// The index text with `rec` added to its last entry as `measured`.
fn attach_measured(index: &str, rec: &MeasuredRecord) -> Option<String> {
    let mut root: Value = serde_json::from_str(index).ok()?;
    let last = root.get_mut("entries")?.as_array_mut()?.last_mut()?;
    last["measured"] = json!({
        "pack_hash": rec.pack_hash,
        "tasks": rec.tasks,
        "successes": rec.successes,
        "success_rate": rec.success_rate,
        "cost_per_task": rec.cost_per_task,
        "measured_by": rec.measured_by,
        "signature": rec.signature,
    });
    serde_json::to_string_pretty(&root).ok()
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
    cfg_from_meta(&meta)
}

/// The listing's config from a record's fields.
fn cfg_from_meta(meta: &serde_json::Map<String, Value>) -> Result<ListingCfg, String> {
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
    let registry_url = match get("registry_url") {
        None => None,
        Some(u) if u.starts_with("https://") => Some(u),
        Some(_) => return Err("team_packs: `registry_url` must be https".to_string()),
    };
    Ok(ListingCfg {
        registry_url,
        allow_insecure: false,
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
            "publish": "publish {pack, url, signing_key, grants?, measured?, opt_in?} → {published: {name, version, hash, signer_key}, trusted, note?, measured_published, measured_reason?}; with opt_in true and a measured record (an eval reply's measured object, signed) it also writes the record beside the entry when its signature verifies and it is for this pack; checks the pack folder like an install (manifest, content hash, secrets scan), signs it with the 64-hex seed in the file bridge file `signing_key`, and appends it to the local index; `url` must be https; `grants` is the only source of the entry's grants; refuses an existing name+version or hash; the key never appears in a reply",
            "fetch": "fetch {name, hash, stage, version?} → {staged, name, version, hash, next}; finds the entry by name and pinned hash in the SIGNED index, refuses a revoked one, downloads the pack from the entry's https url, checks the files against the pin and the secrets scan, and only then writes them under `stage` through the files agent; install the staged folder with `install`",
            "submit": "submit {pack, url, signing_key, grants?} → {listed: true, name, version, hash, signer_key} | {listed: false, status: \"held\", findings: [{path, rule}]}; reviews the pack like publish, holds it on any scan finding, otherwise signs it and POSTs the entry and files to the record's `registry_url` (error `no registry_url` without one); the key never appears in a reply",
            "report": "report {name, version, reason} → {reported: true}; POSTs the report (reason 1 to 2000 characters) to `<registry_url>/report`",
            "takedown": "takedown {name, version, signing_key} → {revoked: {name, version}, pushed}; revokes the version in the local index (re-signed, so the flag is signed) and, with a `registry_url`, POSTs the updated index there; the key never appears in a reply",
            "page": "page → {html}; a static, escaped HTML listing page: ports, grants, evals, measured record and price per pack",
            "browser": "browser {selected?, opt_in?} → {cards, selected?, opt_in, updates, warnings, note?}; the view-model of the web pack browser (updates and their permission diff appear only with opt_in)",
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
                "list" | "show" | "updates" | "page" | "browser" => {
                    match load(&peers, &cfg).await {
                        Err(e) => json!({"error": e}),
                        Ok(loaded) => match t {
                            "list" => {
                                let query = payload.get("query").and_then(|v| v.as_str());
                                let cards = list_view(&loaded.entries, &loaded.installed, query);
                                with_skipped(json!({"packs": cards}), &loaded.skipped)
                            }
                            "show" => {
                                let name =
                                    payload.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                let version = payload.get("version").and_then(|v| v.as_str());
                                match show_view(&loaded.entries, &loaded.installed, name, version) {
                                    Ok(v) => with_skipped(v, &loaded.skipped),
                                    Err(e) => json!({"error": e}),
                                }
                            }
                            "page" => {
                                json!({"html": crate::pack_page::render_listing(&loaded.entries, &loaded.installed)})
                            }
                            "browser" => {
                                let opt_in = payload
                                    .get("opt_in")
                                    .and_then(Value::as_bool)
                                    .unwrap_or(false);
                                let selected = payload.get("selected").and_then(|v| v.as_str());
                                let pins = pins_for(&peers, &cfg, &loaded).await;
                                let vm = crate::pack_browser::browser_view(
                                    &loaded, &pins, selected, opt_in,
                                );
                                with_skipped(
                                    crate::pack_browser::browser_json(&vm),
                                    &loaded.skipped,
                                )
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
                    }
                }
                "publish" => publish(&peers, &cfg, payload).await,
                "fetch" => fetch(&peers, &cfg, payload).await,
                "submit" => submit(&peers, &cfg, payload).await,
                "report" => report(&cfg, payload).await,
                "takedown" => takedown(&peers, &cfg, payload).await,
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
        index: std::sync::Mutex<Option<String>>,
        installed: Vec<String>,
        files: BTreeMap<String, String>,
        /// Every write the fake saw, as (path, bytes).
        writes: std::sync::Mutex<Vec<(String, String)>>,
        /// Every path read.
        reads: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl Peers for Fake {
        async fn ask(&self, _to: &str, p: Value) -> Value {
            match p["type"].as_str() {
                Some("read") => {
                    let path = p["path"].as_str().unwrap_or("");
                    self.reads.lock().unwrap().push(path.to_string());
                    let text = if path == INDEX_PATH {
                        self.index.lock().unwrap().clone()
                    } else {
                        self.files.get(path).cloned()
                    };
                    match text {
                        Some(t) => json!({"content": t}),
                        None => json!({"error": "not found"}),
                    }
                }
                Some("write") => {
                    let path = p["path"].as_str().unwrap_or("").to_string();
                    let bytes = p["bytes"].as_str().unwrap_or("").to_string();
                    if path == INDEX_PATH {
                        *self.index.lock().unwrap() = Some(bytes.clone());
                    }
                    self.writes.lock().unwrap().push((path, bytes));
                    json!({"ok": true})
                }
                Some("list") if p["path"] == "templates" => json!({"entries": self.installed}),
                Some("list") => json!({"error": "not found"}),
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
            registry_url: None,
            allow_insecure: false,
        }
    }

    fn fake(index: Option<&str>, installed: &[&str], files: &[(&str, &str)]) -> Fake {
        Fake {
            index: std::sync::Mutex::new(index.map(str::to_string)),
            writes: Default::default(),
            reads: Default::default(),
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

    /// A file bridge that answers a read as `lines`, as the real one does.
    struct LinesPeer(&'static str);

    #[async_trait::async_trait]
    impl Peers for LinesPeer {
        async fn ask(&self, _to: &str, _p: Value) -> Value {
            let lines: Vec<Value> = self
                .0
                .lines()
                .enumerate()
                .map(|(i, t)| json!({"n": i + 1, "text": t}))
                .collect();
            json!({"lines": lines, "eof": true})
        }
    }

    #[tokio::test]
    async fn pinned_hash_reads_the_bridge_lines_reply() {
        let peer =
            LinesPeer("name: demo\nversion: 1.0.0\nauthor: a\nlicense: MIT\ncontent_hash: abc123");
        let got = pinned_hash(&peer, &cfg_with(vec![]), "demo", "1.0.0").await;
        assert_eq!(got.as_deref(), Some("abc123"));
    }

    #[test]
    fn installed_packs_reads_the_file_bridge_listing() {
        let bridge = json!({"path": "templates", "files": [
            {"name": "demo-1.0.0", "path": "templates/demo-1.0.0", "type": "dir"},
            {"name": "notes.txt", "path": "templates/notes.txt", "type": "file"},
        ]});
        assert_eq!(
            installed_packs(&bridge),
            vec![("demo".to_string(), "1.0.0".to_string())]
        );
        assert_eq!(
            installed_packs(&json!({"entries": ["demo-1.0.0"]})),
            vec![("demo".to_string(), "1.0.0".to_string())]
        );
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

    /// The 64-hex seed the publish tests sign with.
    fn seed_hex() -> String {
        "05".repeat(32)
    }

    /// A fake holding a packed pack at `pack/` (one worker.json with `body`),
    /// a key file at `keys/k` and, when given, an index.
    fn pack_fake(body: &str, index: Option<&str>) -> Fake {
        let hash = pack_manifest::content_hash(&[("worker.json".to_string(), body.into())]);
        let yaml = format!(
            "name: notes\nversion: 1.0.0\nauthor: a\nlicense: MIT\ncontent_hash: {hash}\nports:\n  - \"@connector:github\"\nfiles:\n  - worker.json\n"
        );
        fake(
            index,
            &[],
            &[
                ("pack/pack.yaml", &yaml),
                ("pack/worker.json", body),
                ("keys/k", &format!("{}\n", seed_hex())),
                ("keys/short", "abcdef0123"),
            ],
        )
    }

    fn publish_payload() -> Value {
        json!({"pack": "pack", "url": "https://example.org/notes-1.0.0.tar", "signing_key": "keys/k"})
    }

    fn signer() -> SigningKey {
        SigningKey::from_bytes(&[5u8; 32])
    }

    #[tokio::test]
    async fn publish_signs_and_writes_the_index() {
        let f = pack_fake("{\"name\":\"notes\"}", None);
        let cfg = cfg_with(vec![pubhex(&signer())]);
        let r = publish(&f, &cfg, &publish_payload()).await;
        assert_eq!(r["published"]["name"], json!("notes"));
        assert_eq!(r["published"]["version"], json!("1.0.0"));
        assert_eq!(r["published"]["signer_key"], json!(pubhex(&signer())));
        assert!(r["published"]["hash"].as_str().unwrap().len() == 64);
        assert_eq!(r["trusted"], json!(true));
        assert!(r.get("note").is_none());
        let written = f.index.lock().unwrap().clone().unwrap();
        let idx = pack_registry::Index::parse(&written, &cfg.trusted_keys).unwrap();
        assert_eq!(idx.entries.len(), 1);
        assert_eq!(idx.entries[0].ports, vec!["@connector:github".to_string()]);
        let l = load(&f, &cfg).await.unwrap();
        let cards = list_view(&l.entries, &l.installed, None);
        assert_eq!(cards[0]["name"], json!("notes"));
        assert_eq!(cards[0]["version"], json!("1.0.0"));
    }

    #[tokio::test]
    async fn publish_reply_never_contains_the_key() {
        let f = pack_fake("{}", None);
        let cfg = cfg_with(vec![]);
        let ok = publish(&f, &cfg, &publish_payload()).await;
        assert!(!ok.to_string().contains(&seed_hex()));
        let mut bad = publish_payload();
        bad["url"] = json!("http://example.org/x");
        let e = publish(&f, &cfg, &bad).await;
        assert!(e.get("error").is_some());
        assert!(!e.to_string().contains(&seed_hex()));
        let again = publish(&f, &cfg, &publish_payload()).await;
        assert!(again.get("error").is_some());
        assert!(!again.to_string().contains(&seed_hex()));
    }

    #[tokio::test]
    async fn publish_refuses_a_hash_mismatch() {
        let mut f = pack_fake("{}", None);
        f.files
            .insert("pack/worker.json".into(), "{\"x\":1}".into());
        let r = publish(&f, &cfg_with(vec![]), &publish_payload()).await;
        assert_eq!(r["error"], json!("content hash mismatch"));
        assert!(f.writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn publish_refuses_a_scan_failure() {
        let body = format!("token: {}{}", "AKIA", "0123456789ABCDEFGH");
        let f = pack_fake(&body, None);
        let r = publish(&f, &cfg_with(vec![]), &publish_payload()).await;
        assert_eq!(r["error"], json!("pack refused by scan"));
        assert!(f.writes.lock().unwrap().is_empty());
        assert!(!f.reads.lock().unwrap().iter().any(|p| p == "keys/k"));
    }

    #[tokio::test]
    async fn publish_refuses_an_existing_version() {
        let f = pack_fake("{}", None);
        let cfg = cfg_with(vec![pubhex(&signer())]);
        assert!(publish(&f, &cfg, &publish_payload())
            .await
            .get("error")
            .is_none());
        let before = f.index.lock().unwrap().clone();
        let r = publish(&f, &cfg, &publish_payload()).await;
        assert!(r["error"].as_str().unwrap().contains("already"), "{r}");
        assert_eq!(*f.index.lock().unwrap(), before);
        assert_eq!(f.writes.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn publish_refuses_a_missing_or_bad_key_file() {
        let f = pack_fake("{}", None);
        for path in ["keys/none", "keys/short"] {
            let mut p = publish_payload();
            p["signing_key"] = json!(path);
            let r = publish(&f, &cfg_with(vec![]), &p).await;
            assert!(r.get("error").is_some(), "{path}: {r}");
        }
        assert!(f.writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn publish_refuses_a_non_https_url() {
        let f = pack_fake("{}", None);
        let mut p = publish_payload();
        p["url"] = json!("http://example.org/x.tar");
        let r = publish(&f, &cfg_with(vec![]), &p).await;
        assert!(r.get("error").is_some());
        assert!(f.writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn publish_warns_when_signer_untrusted() {
        let f = pack_fake("{}", None);
        let cfg = cfg_with(vec![]);
        let r = publish(&f, &cfg, &publish_payload()).await;
        assert_eq!(r["trusted"], json!(false));
        assert!(r["note"].as_str().unwrap().contains("trusted_keys"));
        let l = load(&f, &cfg).await.unwrap();
        assert!(l.entries.is_empty());
        assert_eq!(l.skipped.len(), 1);
    }

    #[tokio::test]
    async fn publish_uses_grants_from_the_payload() {
        let f = pack_fake("{}", None);
        let cfg = cfg_with(vec![pubhex(&signer())]);
        let mut p = publish_payload();
        p["grants"] = json!(["files.read"]);
        assert!(publish(&f, &cfg, &p).await.get("error").is_none());
        let l = load(&f, &cfg).await.unwrap();
        let v = show_view(&l.entries, &l.installed, "notes", None).unwrap();
        assert_eq!(v["grants"], json!(["files.read"]));
        let f2 = pack_fake("{}", None);
        let mut bad = publish_payload();
        bad["grants"] = json!([1]);
        assert!(publish(&f2, &cfg, &bad).await.get("error").is_some());
        assert!(f2.writes.lock().unwrap().is_empty());
    }

    #[test]
    fn verb_docs_lists_publish() {
        use fantastic_kernel::bundle::Bundle;
        assert!(ListingBundle.verb_docs().get("publish").is_some());
    }

    #[test]
    fn register_team_registers_listing() {
        let mut reg = fantastic_kernel::BundleRegistry::new();
        crate::register_team(&mut reg);
        assert!(reg.get(HANDLER_MODULE).is_some());
    }

    // ---- hosted registry: fetch, submit, report, takedown, page, browser ----

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn registry_cfg(server: &MockServer, trusted: Vec<String>) -> ListingCfg {
        ListingCfg {
            registry_url: Some(format!("{}/registry", server.uri())),
            allow_insecure: true,
            ..cfg_with(trusted)
        }
    }

    async fn ok_server() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"ok\":true}"))
            .mount(&server)
            .await;
        server
    }

    const BODY: &str = "{\"name\":\"notes\"}";
    const SECRET_BODY: &str = "{\"k\":\"sk-abcdefghijklmnopqrstuvwxyz0123456789\"}";

    #[tokio::test]
    async fn review_holds_a_pack_with_scan_findings() {
        let server = ok_server().await;
        let f = pack_fake(SECRET_BODY, None);
        let cfg = registry_cfg(&server, vec![]);
        let r = submit(&f, &cfg, &publish_payload()).await;
        assert_eq!(r["listed"], json!(false), "{r}");
        assert_eq!(r["status"], json!("held"));
        assert_eq!(r["findings"][0]["path"], json!("worker.json"));
        assert!(!r.to_string().contains("sk-abcdef"));
        assert!(server.received_requests().await.unwrap().is_empty());
        assert!(f.writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn review_lists_a_clean_pack() {
        let server = ok_server().await;
        let f = pack_fake(BODY, None);
        let cfg = registry_cfg(&server, vec![pubhex(&signer())]);
        let r = submit(&f, &cfg, &publish_payload()).await;
        assert_eq!(r["listed"], json!(true), "{r}");
        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 1);
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["entry"]["name"], json!("notes"));
        assert_eq!(body["entry"]["signature"].as_str().unwrap().len(), 128);
        assert_eq!(body["files"]["worker.json"], json!(BODY));
        assert!(body["files"]["pack.yaml"].is_string());
        assert!(!body.to_string().contains(&seed_hex()));
        assert!(f.writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn submit_without_registry_url_is_an_error() {
        let f = pack_fake(BODY, None);
        let r = submit(&f, &cfg_with(vec![]), &publish_payload()).await;
        assert_eq!(r["error"], json!("no registry_url"));
    }

    /// An index holding the published `notes` 1.0.0, signed by `signer()`.
    async fn published_index() -> (Fake, String) {
        let f = pack_fake(BODY, None);
        let cfg = cfg_with(vec![pubhex(&signer())]);
        let r = publish(&f, &cfg, &publish_payload()).await;
        let hash = r["published"]["hash"].as_str().unwrap().to_string();
        (f, hash)
    }

    #[tokio::test]
    async fn takedown_revokes_locally_and_pushes_to_the_hosted_index() {
        let server = ok_server().await;
        let (f, _) = published_index().await;
        let cfg = registry_cfg(&server, vec![pubhex(&signer())]);
        let payload = json!({"name": "notes", "version": "1.0.0", "signing_key": "keys/k"});
        let r = takedown(&f, &cfg, &payload).await;
        assert_eq!(
            r["revoked"],
            json!({"name": "notes", "version": "1.0.0"}),
            "{r}"
        );
        assert_eq!(r["pushed"], json!(true));
        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 1);
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        let pushed =
            pack_registry::Index::parse(&body["index"].to_string(), &[pubhex(&signer())]).unwrap();
        assert!(pushed.entries[0].revoked);
        let local = cfg_with(vec![pubhex(&signer())]);
        let l = load(&f, &local).await.unwrap();
        let v = show_view(&l.entries, &l.installed, "notes", None).unwrap();
        assert_eq!(v["install_decision"]["allow"], json!(false));
    }

    #[tokio::test]
    async fn takedown_never_echoes_the_key() {
        let (f, _) = published_index().await;
        let cfg = cfg_with(vec![pubhex(&signer())]);
        let ok = takedown(
            &f,
            &cfg,
            &json!({"name": "notes", "version": "1.0.0", "signing_key": "keys/k"}),
        )
        .await;
        assert!(ok.get("error").is_none(), "{ok}");
        assert!(!ok.to_string().contains(&seed_hex()));
        let bad = takedown(
            &f,
            &cfg,
            &json!({"name": "notes", "version": "9.9.9", "signing_key": "keys/k"}),
        )
        .await;
        assert!(bad.get("error").is_some());
        assert!(!bad.to_string().contains(&seed_hex()));
        let short = takedown(
            &f,
            &cfg,
            &json!({"name": "notes", "version": "1.0.0", "signing_key": "keys/short"}),
        )
        .await;
        assert!(!short.to_string().contains("abcdef0123"));
    }

    #[tokio::test]
    async fn report_posts_name_version_and_reason() {
        let server = ok_server().await;
        let cfg = registry_cfg(&server, vec![]);
        let r = report(
            &cfg,
            &json!({"name": "notes", "version": "1.0.0", "reason": "steals keys"}),
        )
        .await;
        assert_eq!(r, json!({"reported": true}));
        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs[0].url.path(), "/registry/report");
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(
            body,
            json!({"name": "notes", "version": "1.0.0", "reason": "steals keys"})
        );
        let none = report(
            &cfg_with(vec![]),
            &json!({"name": "n", "version": "1", "reason": "r"}),
        )
        .await;
        assert_eq!(none["error"], json!("no registry_url"));
    }

    #[tokio::test]
    async fn report_refuses_an_empty_reason_or_unsafe_name() {
        let server = ok_server().await;
        let cfg = registry_cfg(&server, vec![]);
        for p in [
            json!({"name": "notes", "version": "1.0.0", "reason": "  "}),
            json!({"name": "../x", "version": "1.0.0", "reason": "r"}),
            json!({"name": "notes", "version": "1.0.0", "reason": "x".repeat(2001)}),
            json!({"name": "notes", "version": "1.0.0"}),
        ] {
            assert!(report(&cfg, &p).await.get("error").is_some(), "{p}");
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    /// A pack served as a download document, and the index pinning it.
    fn download_fixture(revoked: bool) -> (Fake, String, String) {
        let pk = pack_fake(BODY, None);
        let yaml = pk.files["pack/pack.yaml"].clone();
        let hash = pack_manifest::Manifest::parse(&yaml).unwrap().content_hash;
        let k = signer();
        let index = signed_index(vec![signed_entry(
            &k,
            "notes",
            "1.0.0",
            &hash,
            &[],
            revoked,
        )]);
        let doc = json!({"files": {"pack.yaml": yaml, "worker.json": BODY}}).to_string();
        (fake(Some(&index), &[], &[]), hash, doc)
    }

    async fn run_fetch(f: &Fake, doc: &str, hash: &str) -> Value {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/p.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(doc.to_string()))
            .mount(&server)
            .await;
        let cfg = cfg_with(vec![pubhex(&signer())]);
        let uri = format!("{}/p.json", server.uri());
        let payload = json!({"name": "notes", "hash": hash, "stage": "stage"});
        fetch_via(f, &cfg, &payload, |mut e| async move {
            e.url = uri;
            pack_hosted::download_pack_with(&e, true, pack_hosted::MAX_PACK).await
        })
        .await
    }

    #[tokio::test]
    async fn fetch_stages_a_pack_by_hash() {
        let (f, hash, doc) = download_fixture(false);
        let r = run_fetch(&f, &doc, &hash).await;
        assert_eq!(r["staged"], json!("stage"), "{r}");
        let w = f.writes.lock().unwrap();
        let paths: Vec<&str> = w.iter().map(|(p, _)| p.as_str()).collect();
        assert!(paths.contains(&"stage/pack.yaml"), "{paths:?}");
        assert!(paths.contains(&"stage/worker.json"), "{paths:?}");
    }

    #[tokio::test]
    async fn fetch_refuses_a_revoked_entry() {
        let (f, hash, doc) = download_fixture(true);
        let r = run_fetch(&f, &doc, &hash).await;
        assert!(r["error"].as_str().unwrap().contains("revoked"), "{r}");
        assert!(f.writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn fetch_refuses_a_hash_that_is_not_in_the_index() {
        let (f, _, doc) = download_fixture(false);
        let r = run_fetch(&f, &doc, &"0".repeat(64)).await;
        assert!(r.get("error").is_some(), "{r}");
        assert!(f.writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn fetch_refuses_a_download_that_differs_from_the_pin() {
        let (f, hash, _) = download_fixture(false);
        let tampered = json!({"files": {"pack.yaml": f.index.lock().unwrap().clone().unwrap(), "worker.json": "x"}}).to_string();
        let r = run_fetch(&f, &tampered, &hash).await;
        assert!(r.get("error").is_some(), "{r}");
        assert!(f.writes.lock().unwrap().is_empty());
    }

    #[test]
    fn cfg_reads_registry_url_and_refuses_http() {
        let rec = |url: Value| {
            let mut m = serde_json::Map::new();
            m.insert("index_path".into(), json!("i"));
            m.insert("templates_dir".into(), json!("t"));
            m.insert("registry_url".into(), url);
            m
        };
        let ok = cfg_from_meta(&rec(json!("https://r.example/index.json"))).unwrap();
        assert_eq!(
            ok.registry_url.as_deref(),
            Some("https://r.example/index.json")
        );
        assert!(!ok.allow_insecure);
        assert!(cfg_from_meta(&rec(json!("http://r.example"))).is_err());
        assert_eq!(cfg_from_meta(&rec(Value::Null)).unwrap().registry_url, None);
    }

    #[tokio::test]
    async fn load_uses_the_hosted_index_when_registry_url_is_set() {
        let server = MockServer::start().await;
        let k = key(1);
        let index = signed_index(vec![signed_entry(&k, "hosted", "1.0.0", "h1", &[], false)]);
        Mock::given(method("GET"))
            .and(path("/registry"))
            .respond_with(ResponseTemplate::new(200).set_body_string(index))
            .mount(&server)
            .await;
        let cfg = registry_cfg(&server, vec![pubhex(&k)]);
        let local = fake(Some(INDEX), &[], &[]);
        let l = load(&local, &cfg).await.unwrap();
        assert_eq!(l.entries.len(), 1);
        assert_eq!(l.entries[0].name, "hosted");
        assert!(local.reads.lock().unwrap().iter().all(|p| p != INDEX_PATH));
    }

    #[tokio::test]
    async fn page_verb_html_comes_from_the_loaded_entries() {
        let l = loaded(&["alpha"]).await;
        let html = crate::pack_page::render_listing(&l.entries, &l.installed);
        assert!(html.contains("alpha") && html.contains("zeta"));
    }

    #[test]
    fn verb_docs_lists_fetch_submit_report_takedown_page_browser() {
        use fantastic_kernel::bundle::Bundle;
        let docs = ListingBundle.verb_docs();
        for v in ["fetch", "submit", "report", "takedown", "page", "browser"] {
            assert!(docs.get(v).is_some(), "{v}");
        }
    }

    // ---- the opt-in measured record (#41) ----

    /// The hash `pack_fake` packs.
    fn packed_hash(body: &str) -> String {
        pack_manifest::content_hash(&[("worker.json".to_string(), body.into())])
    }

    /// A record for `hash` signed by `k`, in the shape an eval reply carries.
    fn measured_for(k: &SigningKey, hash: &str) -> Value {
        use crate::pack_record::{aggregate, record_json, sign_record, TaskOutcome};
        let o = |succeeded| TaskOutcome {
            pack_hash: hash.to_string(),
            succeeded,
            cost: Some(0.5),
            elapsed_s: 1.0,
        };
        let rec = sign_record(k, aggregate(hash, &[o(true), o(false)], ""));
        let mut v = record_json(&rec);
        v["signature"] = json!(rec.signature);
        v
    }

    fn with_measured_payload(m: Value, opt_in: bool) -> Value {
        let mut p = publish_payload();
        p["measured"] = m;
        if opt_in {
            p["opt_in"] = json!(true);
        }
        p
    }

    fn index_entry(f: &Fake) -> Value {
        let t = f.index.lock().unwrap().clone().unwrap();
        serde_json::from_str::<Value>(&t).unwrap()["entries"][0].clone()
    }

    #[tokio::test]
    async fn publish_without_opt_in_leaves_the_measured_record_out() {
        let body = "{\"name\":\"notes\"}";
        let f = pack_fake(body, None);
        let m = measured_for(&key(9), &packed_hash(body));
        let r = publish(&f, &cfg_with(vec![]), &with_measured_payload(m, false)).await;
        assert!(r.get("error").is_none(), "{r}");
        assert_eq!(r["measured_published"], false, "{r}");
        assert!(
            r["measured_reason"].as_str().unwrap().contains("opt_in"),
            "{r}"
        );
        assert!(index_entry(&f).get("measured").is_none());
        let plain = pack_fake("{}", None);
        let r = publish(&plain, &cfg_with(vec![]), &publish_payload()).await;
        assert_eq!(r["measured_published"], false, "{r}");
    }

    #[tokio::test]
    async fn publish_with_opt_in_and_a_valid_record_writes_it() {
        let body = "{\"name\":\"notes\"}";
        let f = pack_fake(body, None);
        let m = measured_for(&key(9), &packed_hash(body));
        let cfg = cfg_with(vec![pubhex(&signer())]);
        let r = publish(&f, &cfg, &with_measured_payload(m, true)).await;
        assert_eq!(r["measured_published"], true, "{r}");
        assert!(!r.to_string().contains(&seed_hex()));
        let e = index_entry(&f);
        assert_eq!(e["measured"]["tasks"], 2);
        assert_eq!(e["measured"]["success_rate"], 0.5);
        assert_eq!(e["measured"]["measured_by"], pubhex(&key(9)));
        // The entry signature does not cover the record: the entry still verifies.
        let l = load(&f, &cfg).await.unwrap();
        assert!(l.skipped.is_empty(), "{:?}", l.skipped);
        assert_eq!(l.entries[0].measured.as_ref().unwrap().tasks, Some(2));
    }

    #[tokio::test]
    async fn publish_refuses_a_tampered_or_mismatched_record() {
        let body = "{\"name\":\"notes\"}";
        let hash = packed_hash(body);
        let mut tampered = measured_for(&key(9), &hash);
        tampered["tasks"] = json!(99);
        let mut unsigned = measured_for(&key(9), &hash);
        unsigned.as_object_mut().unwrap().remove("signature");
        let other_pack = measured_for(&key(9), "otherhash");
        for (m, why) in [
            (tampered, "signature"),
            (unsigned, "signature"),
            (other_pack, "another pack"),
            (json!({"tasks": 1}), "malformed"),
        ] {
            let f = pack_fake(body, None);
            let r = publish(&f, &cfg_with(vec![]), &with_measured_payload(m, true)).await;
            assert!(r.get("error").is_none(), "the entry is still written: {r}");
            assert_eq!(r["measured_published"], false, "{r}");
            assert!(r["measured_reason"].as_str().unwrap().contains(why), "{r}");
            assert!(index_entry(&f).get("measured").is_none());
        }
    }

    #[tokio::test]
    async fn a_signed_entry_with_a_bad_measured_signature_shows_no_measured() {
        let k = key(3);
        let mut good = signed_entry(&k, "good", "1.0", "h1", &[], false);
        good["measured"] = measured_for(&key(9), "h1");
        let mut bad = signed_entry(&k, "bad", "1.0", "h2", &[], false);
        let mut m = measured_for(&key(9), "h2");
        m["success_rate"] = json!(1.0);
        bad["measured"] = m;
        let mut foreign = signed_entry(&k, "foreign", "1.0", "h3", &[], false);
        foreign["measured"] = measured_for(&key(9), "not-h3");
        let f = fake(Some(&signed_index(vec![good, bad, foreign])), &[], &[]);
        let l = load(&f, &cfg_with(vec![pubhex(&k)])).await.unwrap();
        let by = |n: &str| l.entries.iter().find(|e| e.name == n).unwrap();
        assert!(by("good").measured.is_some());
        assert!(by("bad").measured.is_none());
        assert!(by("foreign").measured.is_none());
    }
}
