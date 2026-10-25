//! Pack registry index: a local index of signed manifests. Packs are pinned
//! by hash, updates are opt-in and show a permission diff, and a revoked
//! signature blocks new installs and warns existing ones. Pure decisions only:
//! no fetching here.
//!
//! Every entry carries an ed25519 signature over its canonical payload
//! (including `revoked`), made by a key on the caller's trusted-key list.
//! Entries with a missing, malformed, bad or untrusted signature are skipped
//! and reported in `skipped`, so a tampered `revoked` flag is never honoured.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// One signed manifest in the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Pack name.
    pub name: String,
    /// Version string, dotted numeric (e.g. `1.2.0`).
    pub version: String,
    /// Content hash the pack is pinned by.
    pub hash: String,
    /// Key that signed this manifest, as lowercase hex (32 bytes).
    pub signer_key: String,
    /// Download location; must be https.
    pub url: String,
    /// Ports the pack exposes.
    pub ports: Vec<String>,
    /// Grants the pack requests.
    pub grants: Vec<String>,
    /// True when the signature has been revoked.
    pub revoked: bool,
    /// hex ed25519 signature over signing_payload
    pub signature: String,
}

/// A parsed registry index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
    /// All well-formed manifests in the index.
    pub entries: Vec<Entry>,
    /// One reason per entry that was skipped, like `entry 2 (name): ...`.
    pub skipped: Vec<String>,
}

/// Permission changes between two versions of a pack.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Diff {
    /// Ports present in the new version only.
    pub added_ports: Vec<String>,
    /// Ports present in the old version only.
    pub removed_ports: Vec<String>,
    /// Grants present in the new version only.
    pub added_grants: Vec<String>,
    /// Grants present in the old version only.
    pub removed_grants: Vec<String>,
}

/// Outcome of an install check.
#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    /// The install may proceed.
    Allow,
    /// The install is refused, with the reason.
    Refuse(String),
}

impl Index {
    /// Parse an index document. A document that is not JSON, or has no
    /// `entries` list, is an error. A malformed entry (bad field, non-https
    /// url, or a version that is not dotted numbers) is skipped and reported
    /// in `skipped`, so one bad entry does not make the whole index unusable.
    /// An entry whose signature is missing, malformed, bad or made by a key
    /// not in `trusted_keys` is skipped the same way.
    pub fn parse(json: &str, trusted_keys: &[String]) -> Result<Self, String> {
        let root: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let items = root
            .get("entries")
            .and_then(Value::as_array)
            .ok_or("index must have an `entries` list")?;
        let mut entries = Vec::with_capacity(items.len());
        let mut skipped = Vec::new();
        for (i, item) in items.iter().enumerate() {
            match parse_entry(item)
                .and_then(|entry| verify_entry(&entry, trusted_keys).map(|()| entry))
            {
                Ok(entry) => entries.push(entry),
                Err(reason) => {
                    let name = item.get("name").and_then(Value::as_str).unwrap_or("?");
                    skipped.push(format!("entry {} ({name}): {reason}", i + 1));
                }
            }
        }
        Ok(Index { entries, skipped })
    }
}

/// Build one entry from its JSON object, or say why it is malformed.
fn parse_entry(item: &Value) -> Result<Entry, String> {
    let version = str_field(item, "version")?;
    parse_version(&version)?;
    let url = str_field(item, "url")?;
    check_https(&url)?;
    Ok(Entry {
        name: str_field(item, "name")?,
        version,
        hash: str_field(item, "hash")?,
        signer_key: str_field(item, "signer_key")?,
        url,
        ports: str_list(item, "ports")?,
        grants: str_list(item, "grants")?,
        revoked: item
            .get("revoked")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        signature: str_field(item, "signature")?,
    })
}

/// The canonical bytes a signature covers: a JSON object with sorted keys and
/// every field except `signature`. The output is deterministic.
pub fn signing_payload(e: &Entry) -> Vec<u8> {
    let mut m: BTreeMap<&str, Value> = BTreeMap::new();
    m.insert("name", json!(e.name));
    m.insert("version", json!(e.version));
    m.insert("hash", json!(e.hash));
    m.insert("signer_key", json!(e.signer_key));
    m.insert("url", json!(e.url));
    m.insert("ports", json!(e.ports));
    m.insert("grants", json!(e.grants));
    m.insert("revoked", json!(e.revoked));
    serde_json::to_vec(&m).unwrap_or_default()
}

/// Check the entry's signature against its own payload. The signer must be
/// on `trusted_keys`, and the signature must verify strictly.
pub fn verify_entry(e: &Entry, trusted_keys: &[String]) -> Result<(), String> {
    if !trusted_keys.iter().any(|k| k == &e.signer_key) {
        return Err("untrusted signer".to_string());
    }
    let key_bytes: [u8; 32] = decode_hex(&e.signer_key)?
        .try_into()
        .map_err(|_| "signer key must be 32 bytes".to_string())?;
    let key = VerifyingKey::from_bytes(&key_bytes)
        .map_err(|_| "signer key is not a valid ed25519 key".to_string())?;
    let sig_bytes: [u8; 64] = decode_hex(&e.signature)?
        .try_into()
        .map_err(|_| "signature must be 64 bytes".to_string())?;
    let sig = Signature::from_bytes(&sig_bytes);
    key.verify_strict(&signing_payload(e), &sig)
        .map_err(|_| "bad signature".to_string())
}

/// Sign an entry: set `signer_key` to the public key of `key` (whatever the
/// entry carried) and fill `signature` over the canonical payload. Pure.
pub fn sign_entry(key: &SigningKey, mut unsigned: Entry) -> Entry {
    unsigned.signer_key = hex::encode(key.verifying_key().to_bytes());
    let sig = key.sign(&signing_payload(&unsigned));
    unsigned.signature = hex::encode(sig.to_bytes());
    unsigned
}

/// A signing key from its 64-hex seed text (surrounding whitespace is
/// trimmed). The error is a fixed message and never echoes the input.
pub fn signing_key_from_hex(text: &str) -> Result<SigningKey, String> {
    let bad = || "signing key must be 64 hex characters".to_string();
    let bytes = hex::decode(text.trim()).map_err(|_| bad())?;
    let seed: [u8; 32] = bytes.try_into().map_err(|_| bad())?;
    Ok(SigningKey::from_bytes(&seed))
}

/// An entry as the index holds it: every field, signature included.
pub fn entry_json(e: &Entry) -> Value {
    json!({
        "name": e.name,
        "version": e.version,
        "hash": e.hash,
        "signer_key": e.signer_key,
        "url": e.url,
        "ports": e.ports,
        "grants": e.grants,
        "revoked": e.revoked,
        "signature": e.signature,
    })
}

/// Append a signed entry to an index document (`None` or empty starts a new
/// one) and return the pretty-printed JSON. Refuses a non-https url, a version
/// that is not dotted numbers, an entry whose own signature does not verify
/// against its own signer key, a duplicate (name, version) and a duplicate
/// hash. Existing entries are kept as they are, including ones this build
/// would skip.
pub fn add_entry(index_json: Option<&str>, entry: Entry) -> Result<String, String> {
    check_https(&entry.url)?;
    parse_version(&entry.version)?;
    verify_entry(&entry, std::slice::from_ref(&entry.signer_key))?;
    let mut root: Value = match index_json.map(str::trim).filter(|t| !t.is_empty()) {
        None => json!({"entries": []}),
        Some(t) => serde_json::from_str(t).map_err(|e| e.to_string())?,
    };
    let items = root
        .get_mut("entries")
        .and_then(Value::as_array_mut)
        .ok_or("index must have an `entries` list")?;
    let field = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    for item in items.iter() {
        if field(item, "name").as_deref() == Some(entry.name.as_str())
            && field(item, "version").as_deref() == Some(entry.version.as_str())
        {
            return Err(format!(
                "{} {} is already in the index",
                entry.name, entry.version
            ));
        }
        if field(item, "hash").as_deref() == Some(entry.hash.as_str()) {
            return Err("an entry with this hash is already in the index".to_string());
        }
    }
    items.push(entry_json(&entry));
    serde_json::to_string_pretty(&root).map_err(|e| e.to_string())
}

/// Revoke one version in an index document: set `revoked` on the entry for
/// `name` and `version` and sign it again with `key`, so the signature covers
/// the flag. Other entries are kept as they are. An unknown name or version is
/// an error. Returns the updated, pretty-printed document.
pub fn revoke_entry(
    index_json: &str,
    name: &str,
    version: &str,
    key: &SigningKey,
) -> Result<String, String> {
    let mut root: Value = serde_json::from_str(index_json).map_err(|e| e.to_string())?;
    let items = root
        .get_mut("entries")
        .and_then(Value::as_array_mut)
        .ok_or("index must have an `entries` list")?;
    let field = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    let item = items
        .iter_mut()
        .find(|v| {
            field(v, "name").as_deref() == Some(name)
                && field(v, "version").as_deref() == Some(version)
        })
        .ok_or_else(|| format!("no entry for {name} {version}"))?;
    let mut entry = parse_entry(item)?;
    entry.revoked = true;
    *item = entry_json(&sign_entry(key, entry));
    serde_json::to_string_pretty(&root).map_err(|e| e.to_string())
}

fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
    hex::decode(s).map_err(|_| "key or signature is not hex".to_string())
}

fn str_field(v: &Value, key: &str) -> Result<String, String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("entry field `{key}` must be a string"))
}

fn str_list(v: &Value, key: &str) -> Result<Vec<String>, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|i| {
                i.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("`{key}` items must be strings"))
            })
            .collect(),
        Some(_) => Err(format!("`{key}` must be a list")),
    }
}

fn check_https(url: &str) -> Result<(), String> {
    if url.starts_with("https://") {
        Ok(())
    } else {
        Err(format!("url must be https: {url}"))
    }
}

/// Split a dotted numeric version (`1.2.0`) into its numeric parts. Rejects an
/// empty string, an empty part, and any part that is not all ASCII digits.
fn parse_version(v: &str) -> Result<Vec<u64>, String> {
    if v.is_empty() {
        return Err("version must not be empty".to_string());
    }
    v.split('.')
        .map(|p| {
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return Err(format!("version must be dotted numbers: `{v}`"));
            }
            p.parse::<u64>()
                .map_err(|_| format!("version part too large: `{v}`"))
        })
        .collect()
}

/// Sort key for a version. Entries are validated at parse, so the fallback
/// is not reached for indexed entries.
pub fn version_key(v: &str) -> Vec<u64> {
    parse_version(v).unwrap_or_default()
}

/// The entry for exactly this name and pinned hash, or an error.
pub fn resolve<'a>(index: &'a Index, name: &str, pinned_hash: &str) -> Result<&'a Entry, String> {
    index
        .entries
        .iter()
        .find(|e| e.name == name && e.hash == pinned_hash)
        .ok_or_else(|| format!("no entry for {name} at pinned hash {pinned_hash}"))
}

/// The newest non-revoked version of the installed pack. None without opt-in,
/// or when the newest is already the installed hash.
pub fn update_candidate<'a>(
    index: &'a Index,
    installed: (&str, &str),
    opt_in: bool,
) -> Option<&'a Entry> {
    if !opt_in {
        return None;
    }
    let (name, hash) = installed;
    let best = index
        .entries
        .iter()
        .filter(|e| e.name == name && !e.revoked)
        .max_by_key(|e| version_key(&e.version))?;
    if best.hash == hash {
        None
    } else {
        Some(best)
    }
}

fn difference(from: &[String], minus: &[String]) -> Vec<String> {
    let minus: BTreeSet<&str> = minus.iter().map(String::as_str).collect();
    from.iter()
        .filter(|x| !minus.contains(x.as_str()))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// What moving from `old` to `new` adds and removes in ports and grants.
pub fn permission_diff(old: &Entry, new: &Entry) -> Diff {
    Diff {
        added_ports: difference(&new.ports, &old.ports),
        removed_ports: difference(&old.ports, &new.ports),
        added_grants: difference(&new.grants, &old.grants),
        removed_grants: difference(&old.grants, &new.grants),
    }
}

/// Whether a new install of this entry is allowed.
pub fn install_decision(entry: &Entry) -> Decision {
    if entry.revoked {
        return Decision::Refuse("revoked".to_string());
    }
    if check_https(&entry.url).is_err() {
        return Decision::Refuse("non-https url".to_string());
    }
    Decision::Allow
}

/// One warning per installed (name, hash) whose index entry is revoked.
pub fn installed_warnings(index: &Index, installed: &[(&str, &str)]) -> Vec<String> {
    installed
        .iter()
        .filter_map(|(name, hash)| {
            index
                .entries
                .iter()
                .find(|e| e.name == *name && e.hash == *hash && e.revoked)
                .map(|e| {
                    format!(
                        "{} {} is revoked (signer {}): stop using it",
                        e.name, e.version, e.signer_key
                    )
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    /// A signing key from a fixed seed, so the fixtures are deterministic.
    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn pubhex(k: &SigningKey) -> String {
        hex::encode(k.verifying_key().to_bytes())
    }

    /// Two signers, and the trusted-key list that holds both.
    fn two() -> (SigningKey, SigningKey, Vec<String>) {
        let (k1, k2) = (key(1), key(2));
        let trusted = vec![pubhex(&k1), pubhex(&k2)];
        (k1, k2, trusted)
    }

    /// Unsigned notes fields for one version.
    fn notes(version: &str, hash: &str, ports: &[&str], grants: &[&str]) -> Value {
        json!({
            "name": "notes",
            "version": version,
            "hash": hash,
            "url": format!("https://example.org/notes-{version}.tar"),
            "ports": ports,
            "grants": grants,
        })
    }

    fn revoked(mut v: Value) -> Value {
        v["revoked"] = json!(true);
        v
    }

    /// Sign the entry built from `fields` with `k`. The signer_key is set to
    /// `k`'s public key and the hex signature is added to the JSON object.
    fn signed(k: &SigningKey, mut fields: Value) -> Value {
        let text = |key: &str| {
            fields
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let list = |key: &str| -> Vec<String> {
            fields
                .get(key)
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        let entry = Entry {
            name: text("name"),
            version: text("version"),
            hash: text("hash"),
            signer_key: pubhex(k),
            url: text("url"),
            ports: list("ports"),
            grants: list("grants"),
            revoked: fields
                .get("revoked")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            signature: String::new(),
        };
        let sig = k.sign(&signing_payload(&entry));
        let obj = fields.as_object_mut().expect("fields are a JSON object");
        obj.insert("signer_key".into(), json!(entry.signer_key));
        obj.insert("signature".into(), json!(hex::encode(sig.to_bytes())));
        fields
    }

    fn index_doc(entries: Vec<Value>) -> String {
        json!({ "entries": entries }).to_string()
    }

    /// Four signed notes versions. 1.3.0 is revoked and signed by k2.
    fn fixture(k1: &SigningKey, k2: &SigningKey) -> String {
        index_doc(vec![
            signed(k1, notes("1.0.0", "h1", &["read"], &["files.read"])),
            signed(
                k1,
                notes("1.2.0", "h2", &["read", "write"], &["files.read"]),
            ),
            signed(k1, notes("1.1.0", "h3", &["read"], &["files.read"])),
            signed(k2, revoked(notes("1.3.0", "h4", &[], &[]))),
        ])
    }

    #[test]
    fn index_parses() {
        let (k1, k2, t) = two();
        let idx = Index::parse(&fixture(&k1, &k2), &t).unwrap();
        assert_eq!(idx.entries.len(), 4, "{:?}", idx.skipped);
        assert!(idx.skipped.is_empty());
        assert_eq!(idx.entries[0].ports, vec!["read".to_string()]);
        assert!(idx.entries[3].revoked);
    }

    #[test]
    fn one_bad_entry_does_not_fail_the_index() {
        let k1 = key(1);
        let t = vec![pubhex(&k1)];
        let json = index_doc(vec![
            signed(&k1, notes("1.0.0", "h1", &[], &[])),
            signed(
                &k1,
                json!({"name":"bad","version":"1.0.0","hash":"h2","url":"http://example.org/x.tar"}),
            ),
            signed(
                &k1,
                json!({"name":"good2","version":"2.0.0","hash":"h3","url":"https://example.org/b.tar"}),
            ),
        ]);
        let idx = Index::parse(&json, &t).unwrap();
        assert_eq!(idx.entries.len(), 2);
        assert!(idx.entries.iter().any(|e| e.name == "notes"));
        assert!(idx.entries.iter().any(|e| e.name == "good2"));
        assert_eq!(idx.skipped.len(), 1, "{:?}", idx.skipped);
        assert!(idx.skipped[0].contains("bad"), "{:?}", idx.skipped);
        assert!(idx.skipped[0].contains("https"), "{:?}", idx.skipped);
        assert!(idx.skipped[0].contains("entry 2"), "{:?}", idx.skipped);
    }

    #[test]
    fn a_bad_document_is_still_an_error() {
        assert!(Index::parse("not json", &[]).is_err());
        assert!(Index::parse(r#"{"packs":[]}"#, &[]).is_err());
    }

    #[test]
    fn non_numeric_version_is_rejected_at_parse() {
        let k1 = key(1);
        let t = vec![pubhex(&k1)];
        let json = index_doc(vec![
            signed(&k1, notes("1.2.0", "h1", &[], &[])),
            signed(
                &k1,
                json!({"name":"notes","version":"1.2.0-rc1","hash":"h2","url":"https://example.org/b.tar"}),
            ),
            signed(
                &k1,
                json!({"name":"notes","version":"1.x","hash":"h3","url":"https://example.org/c.tar"}),
            ),
            signed(
                &k1,
                json!({"name":"notes","version":"","hash":"h4","url":"https://example.org/d.tar"}),
            ),
            signed(
                &k1,
                json!({"name":"notes","version":"1..2","hash":"h5","url":"https://example.org/e.tar"}),
            ),
        ]);
        let idx = Index::parse(&json, &t).unwrap();
        assert_eq!(idx.entries.len(), 1);
        assert_eq!(idx.skipped.len(), 4, "{:?}", idx.skipped);
        assert!(idx.skipped.iter().all(|s| s.contains("version")));
        let newest = update_candidate(&idx, ("notes", "h0"), true).unwrap();
        assert_eq!(newest.version, "1.2.0");
        assert!(update_candidate(&idx, ("notes", "h1"), true).is_none());
    }

    #[test]
    fn update_candidate_orders_numerically() {
        let k1 = key(1);
        let t = vec![pubhex(&k1)];
        let json = index_doc(vec![
            signed(&k1, notes("1.9.0", "a", &[], &[])),
            signed(&k1, notes("1.10.0", "b", &[], &[])),
        ]);
        let idx = Index::parse(&json, &t).unwrap();
        let newest = update_candidate(&idx, ("notes", "z"), true).unwrap();
        assert_eq!(newest.version, "1.10.0");
    }

    #[test]
    fn resolve_matches_only_the_pinned_hash() {
        let (k1, k2, t) = two();
        let idx = Index::parse(&fixture(&k1, &k2), &t).unwrap();
        assert_eq!(resolve(&idx, "notes", "h2").unwrap().version, "1.2.0");
        assert!(resolve(&idx, "notes", "h9").is_err());
        assert!(resolve(&idx, "other", "h2").is_err());
    }

    #[test]
    fn no_update_without_opt_in() {
        let (k1, k2, t) = two();
        let idx = Index::parse(&fixture(&k1, &k2), &t).unwrap();
        assert!(update_candidate(&idx, ("notes", "h1"), false).is_none());
    }

    #[test]
    fn update_picks_the_newest_unrevoked() {
        let (k1, k2, t) = two();
        let idx = Index::parse(&fixture(&k1, &k2), &t).unwrap();
        let candidate = update_candidate(&idx, ("notes", "h1"), true).unwrap();
        assert_eq!(candidate.version, "1.2.0");
        assert!(update_candidate(&idx, ("notes", "h2"), true).is_none());
    }

    #[test]
    fn permission_diff_lists_added_and_removed() {
        let (k1, k2, t) = two();
        let idx = Index::parse(&fixture(&k1, &k2), &t).unwrap();
        let old = resolve(&idx, "notes", "h1").unwrap();
        let new = resolve(&idx, "notes", "h2").unwrap();
        let up = permission_diff(old, new);
        assert_eq!(up.added_ports, vec!["write".to_string()]);
        assert!(up.removed_ports.is_empty());
        assert!(up.added_grants.is_empty());
        let down = permission_diff(new, old);
        assert_eq!(down.removed_ports, vec!["write".to_string()]);
        assert!(down.added_ports.is_empty());
    }

    #[test]
    fn a_revoked_entry_refuses_new_installs() {
        let (k1, k2, t) = two();
        let idx = Index::parse(&fixture(&k1, &k2), &t).unwrap();
        let revoked = resolve(&idx, "notes", "h4").unwrap();
        assert_eq!(
            install_decision(revoked),
            Decision::Refuse("revoked".to_string())
        );
        let fine = resolve(&idx, "notes", "h2").unwrap();
        assert_eq!(install_decision(fine), Decision::Allow);
    }

    #[test]
    fn an_installed_revoked_pack_gets_a_warning() {
        let (k1, k2, t) = two();
        let idx = Index::parse(&fixture(&k1, &k2), &t).unwrap();
        let warnings = installed_warnings(&idx, &[("notes", "h4"), ("notes", "h1")]);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("revoked"));
    }

    #[test]
    fn tampered_revoked_flag_is_skipped() {
        let k1 = key(1);
        let t = vec![pubhex(&k1)];
        // Signed as not revoked, then the flag is flipped without re-signing.
        let mut e = signed(&k1, notes("1.2.0", "h2", &["read"], &[]));
        e["revoked"] = json!(true);
        let idx = Index::parse(&index_doc(vec![e]), &t).unwrap();
        assert!(idx.entries.is_empty());
        assert_eq!(idx.skipped.len(), 1, "{:?}", idx.skipped);
        assert!(
            idx.skipped[0].contains("bad signature"),
            "{:?}",
            idx.skipped
        );
        assert!(resolve(&idx, "notes", "h2").is_err());
    }

    #[test]
    fn tampered_unrevoke_cannot_resurrect_update() {
        let (k1, k2, t) = two();
        let mut tampered = signed(&k2, revoked(notes("1.3.0", "h4", &[], &[])));
        tampered["revoked"] = json!(false);
        let json = index_doc(vec![
            signed(&k1, notes("1.0.0", "h1", &["read"], &[])),
            tampered,
        ]);
        let idx = Index::parse(&json, &t).unwrap();
        assert_eq!(idx.entries.len(), 1, "{:?}", idx.entries);
        assert_eq!(idx.entries[0].hash, "h1");
        assert_eq!(idx.skipped.len(), 1, "{:?}", idx.skipped);
        assert!(
            idx.skipped[0].contains("bad signature"),
            "{:?}",
            idx.skipped
        );
        assert!(update_candidate(&idx, ("notes", "h1"), true).is_none());
        assert!(installed_warnings(&idx, &[("notes", "h4")]).is_empty());
    }

    #[test]
    fn untrusted_signer_is_skipped() {
        let (k1, k3) = (key(1), key(3));
        let t = vec![pubhex(&k1)];
        let json = index_doc(vec![
            signed(&k3, notes("1.4.0", "h5", &[], &[])),
            signed(&k1, notes("1.2.0", "h2", &[], &[])),
        ]);
        let idx = Index::parse(&json, &t).unwrap();
        assert_eq!(idx.entries.len(), 1, "{:?}", idx.entries);
        assert_eq!(idx.entries[0].hash, "h2");
        assert_eq!(idx.skipped.len(), 1, "{:?}", idx.skipped);
        assert!(idx.skipped[0].contains("untrusted"), "{:?}", idx.skipped);
    }

    #[test]
    fn missing_or_malformed_signature_is_skipped() {
        let (k1, k3) = (key(1), key(3));
        let t = vec![pubhex(&k1)];
        let no_sig = {
            let mut e = signed(&k1, notes("1.0.0", "h1", &[], &[]));
            e.as_object_mut().unwrap().remove("signature");
            e
        };
        let not_hex = {
            let mut e = signed(&k1, notes("1.1.0", "h2", &[], &[]));
            e["signature"] = json!("zz-not-hex");
            e
        };
        let wrong_len = {
            let mut e = signed(&k1, notes("1.2.0", "h3", &[], &[]));
            e["signature"] = json!(hex::encode([0u8; 32]));
            e
        };
        let other_key = {
            // Made by k3 but claims k1 as the signer.
            let mut e = signed(&k3, notes("1.3.0", "h4", &[], &[]));
            e["signer_key"] = json!(pubhex(&k1));
            e
        };
        let json = index_doc(vec![no_sig, not_hex, wrong_len, other_key]);
        let idx = Index::parse(&json, &t).expect("a bad signature is not a parse error");
        assert!(idx.entries.is_empty(), "{:?}", idx.entries);
        assert_eq!(idx.skipped.len(), 4, "{:?}", idx.skipped);
    }

    #[test]
    fn valid_signed_revoked_entry_is_honoured() {
        let k1 = key(1);
        let t = vec![pubhex(&k1)];
        let json = index_doc(vec![signed(&k1, revoked(notes("1.2.0", "h2", &[], &[])))]);
        let idx = Index::parse(&json, &t).unwrap();
        assert_eq!(idx.entries.len(), 1, "{:?}", idx.skipped);
        let e = resolve(&idx, "notes", "h2").unwrap();
        assert_eq!(install_decision(e), Decision::Refuse("revoked".to_string()));
        assert_eq!(installed_warnings(&idx, &[("notes", "h2")]).len(), 1);
        assert!(update_candidate(&idx, ("notes", "h1"), true).is_none());
    }

    /// An unsigned entry for the publish tests.
    fn unsigned(name: &str, version: &str, hash: &str) -> Entry {
        Entry {
            name: name.into(),
            version: version.into(),
            hash: hash.into(),
            signer_key: "ignored".into(),
            url: format!("https://example.org/{name}-{version}.tar"),
            ports: vec!["@connector:github".into()],
            grants: vec!["files.read".into()],
            revoked: false,
            signature: String::new(),
        }
    }

    #[test]
    fn sign_entry_verifies() {
        let k = key(7);
        let e = sign_entry(&k, unsigned("p", "1.0.0", "h"));
        let trusted = vec![pubhex(&k)];
        assert_eq!(verify_entry(&e, &trusted), Ok(()));
        let mut t = e.clone();
        t.hash = "other".into();
        assert!(verify_entry(&t, &trusted).is_err());
        let mut t = e;
        t.revoked = true;
        assert!(verify_entry(&t, &trusted).is_err());
    }

    #[test]
    fn sign_entry_sets_signer_key() {
        let k = key(7);
        let e = sign_entry(&k, unsigned("p", "1.0.0", "h"));
        assert_eq!(e.signer_key, pubhex(&k));
    }

    #[test]
    fn signing_key_from_hex_accepts_64_hex_and_trims() {
        let k = signing_key_from_hex(&format!("{}\n", "07".repeat(32))).unwrap();
        assert_eq!(pubhex(&k), pubhex(&key(7)));
    }

    #[test]
    fn signing_key_from_hex_rejects_bad_input_without_echo() {
        for bad in [
            "abcd1234",
            "zz".repeat(32).as_str(),
            "07".repeat(33).as_str(),
        ] {
            let e = signing_key_from_hex(bad).unwrap_err();
            assert!(!e.contains(bad), "{e}");
        }
    }

    #[test]
    fn add_entry_creates_a_new_index() {
        let k = key(7);
        let e = sign_entry(&k, unsigned("p", "1.0.0", "h"));
        let text = add_entry(None, e).unwrap();
        let idx = Index::parse(&text, &[pubhex(&k)]).unwrap();
        assert_eq!(idx.entries.len(), 1);
        assert!(idx.skipped.is_empty());
        assert!(add_entry(Some("  "), sign_entry(&k, unsigned("q", "1.0.0", "h2"))).is_ok());
    }

    #[test]
    fn add_entry_appends_and_round_trips() {
        let (k1, k2, trusted) = two();
        let before = fixture(&k1, &k2);
        let e = sign_entry(&k1, unsigned("other", "1.0.0", "hz"));
        let text = add_entry(Some(&before), e.clone()).unwrap();
        let idx = Index::parse(&text, &trusted).unwrap();
        assert_eq!(idx.entries.len(), 5);
        assert_eq!(idx.entries[4], e);
        let old = Index::parse(&before, &trusted).unwrap();
        assert_eq!(idx.entries[..4], old.entries[..]);
    }

    #[test]
    fn add_entry_refuses_duplicate_name_version() {
        let k = key(7);
        let first = add_entry(None, sign_entry(&k, unsigned("p", "1.0.0", "h"))).unwrap();
        let again = sign_entry(&k, unsigned("p", "1.0.0", "h2"));
        assert!(add_entry(Some(&first), again).is_err());
    }

    #[test]
    fn add_entry_refuses_duplicate_hash() {
        let k = key(7);
        let first = add_entry(None, sign_entry(&k, unsigned("p", "1.0.0", "h"))).unwrap();
        assert!(add_entry(Some(&first), sign_entry(&k, unsigned("q", "1.0.0", "h"))).is_err());
        assert!(add_entry(Some(&first), sign_entry(&k, unsigned("p", "1.1.0", "h"))).is_err());
    }

    #[test]
    fn add_entry_refuses_non_https_and_bad_version() {
        let k = key(7);
        let mut e = unsigned("p", "1.0.0", "h");
        e.url = "http://example.org/p.tar".into();
        assert!(add_entry(None, sign_entry(&k, e)).is_err());
        let e = unsigned("p", "1.2.0-rc1", "h");
        assert!(add_entry(None, sign_entry(&k, e)).is_err());
    }

    #[test]
    fn add_entry_refuses_a_bad_signature() {
        let k = key(7);
        let mut e = sign_entry(&k, unsigned("p", "1.0.0", "h"));
        e.hash = "tampered".into();
        assert!(add_entry(None, e).is_err());
    }

    #[test]
    fn add_entry_keeps_unparseable_existing_entries() {
        let k = key(7);
        let doc = r#"{"entries":[{"name":"broken","version":"x"}]}"#;
        let text = add_entry(Some(doc), sign_entry(&k, unsigned("p", "1.0.0", "h"))).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["entries"][0], json!({"name":"broken","version":"x"}));
        let idx = Index::parse(&text, &[pubhex(&k)]).unwrap();
        assert_eq!(idx.entries.len(), 1);
        assert_eq!(idx.skipped.len(), 1);
    }

    #[test]
    fn add_entry_rejects_an_index_without_entries() {
        let e = sign_entry(&key(7), unsigned("p", "1.0.0", "h"));
        assert!(add_entry(Some("{}"), e.clone()).is_err());
        assert!(add_entry(Some("not json"), e).is_err());
    }

    #[test]
    fn revoke_entry_resigns_and_is_honoured() {
        let k = key(1);
        let idx = serde_json::to_string(&json!({"entries": [
            signed(&k, notes("1.0.0", "h1", &[], &[])),
            signed(&k, notes("1.1.0", "h2", &[], &[])),
        ]}))
        .unwrap();
        let out = revoke_entry(&idx, "notes", "1.1.0", &k).unwrap();
        let parsed = Index::parse(&out, &[pubhex(&k)]).unwrap();
        assert!(parsed.skipped.is_empty(), "{:?}", parsed.skipped);
        let e = resolve(&parsed, "notes", "h2").unwrap();
        assert!(e.revoked);
        assert!(matches!(install_decision(e), Decision::Refuse(_)));
        assert!(!resolve(&parsed, "notes", "h1").unwrap().revoked);
    }

    #[test]
    fn revoke_entry_unknown_version_is_an_error() {
        let k = key(1);
        let idx = serde_json::to_string(
            &json!({"entries": [signed(&k, notes("1.0.0", "h1", &[], &[]))]}),
        )
        .unwrap();
        assert!(revoke_entry(&idx, "notes", "9.9.9", &k).is_err());
        assert!(revoke_entry(&idx, "other", "1.0.0", &k).is_err());
        assert!(revoke_entry("{}", "notes", "1.0.0", &k).is_err());
    }

    #[test]
    fn revoke_entry_with_untrusted_key_is_skipped_on_parse() {
        let (k1, k2) = (key(1), key(2));
        let idx = serde_json::to_string(
            &json!({"entries": [signed(&k1, notes("1.0.0", "h1", &[], &[]))]}),
        )
        .unwrap();
        let out = revoke_entry(&idx, "notes", "1.0.0", &k2).unwrap();
        let parsed = Index::parse(&out, &[pubhex(&k1)]).unwrap();
        assert!(parsed.entries.is_empty());
        assert_eq!(parsed.skipped.len(), 1);
    }
}
