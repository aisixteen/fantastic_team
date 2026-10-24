//! Pack registry index: a local index of signed manifests. Packs are pinned
//! by hash, updates are opt-in and show a permission diff, and a revoked
//! signature blocks new installs and warns existing ones. Pure decisions only:
//! no fetching and no signature verification here.

use serde_json::Value;
use std::collections::BTreeSet;

/// One signed manifest in the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Pack name.
    pub name: String,
    /// Version string, dotted numeric (e.g. `1.2.0`).
    pub version: String,
    /// Content hash the pack is pinned by.
    pub hash: String,
    /// Key that signed this manifest.
    pub signer_key: String,
    /// Download location; must be https.
    pub url: String,
    /// Ports the pack exposes.
    pub ports: Vec<String>,
    /// Grants the pack requests.
    pub grants: Vec<String>,
    /// True when the signature has been revoked.
    pub revoked: bool,
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
    pub fn parse(json: &str) -> Result<Self, String> {
        let root: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let items = root
            .get("entries")
            .and_then(Value::as_array)
            .ok_or("index must have an `entries` list")?;
        let mut entries = Vec::with_capacity(items.len());
        let mut skipped = Vec::new();
        for (i, item) in items.iter().enumerate() {
            match parse_entry(item) {
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
    })
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
fn version_key(v: &str) -> Vec<u64> {
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

    const INDEX: &str = r#"{"entries":[
 {"name":"notes","version":"1.0.0","hash":"h1","signer_key":"k1","url":"https://example.org/notes-1.0.0.tar","ports":["read"],"grants":["files.read"]},
 {"name":"notes","version":"1.2.0","hash":"h2","signer_key":"k1","url":"https://example.org/notes-1.2.0.tar","ports":["read","write"],"grants":["files.read"]},
 {"name":"notes","version":"1.1.0","hash":"h3","signer_key":"k1","url":"https://example.org/notes-1.1.0.tar","ports":["read"],"grants":["files.read"]},
 {"name":"notes","version":"1.3.0","hash":"h4","signer_key":"k2","url":"https://example.org/notes-1.3.0.tar","revoked":true}
]}"#;

    #[test]
    fn index_parses() {
        let idx = Index::parse(INDEX).unwrap();
        assert_eq!(idx.entries.len(), 4);
        assert_eq!(idx.entries[0].ports, vec!["read".to_string()]);
        assert!(idx.entries[3].revoked);
    }

    #[test]
    fn one_bad_entry_does_not_fail_the_index() {
        let json = r#"{"entries":[
 {"name":"good1","version":"1.0.0","hash":"h1","signer_key":"k","url":"https://example.org/a.tar"},
 {"name":"bad","version":"1.0.0","hash":"h2","signer_key":"k","url":"http://example.org/x.tar"},
 {"name":"good2","version":"2.0.0","hash":"h3","signer_key":"k","url":"https://example.org/b.tar"}
]}"#;
        let idx = Index::parse(json).unwrap();
        assert_eq!(idx.entries.len(), 2);
        assert!(idx.entries.iter().any(|e| e.name == "good1"));
        assert!(idx.entries.iter().any(|e| e.name == "good2"));
        assert_eq!(idx.skipped.len(), 1, "{:?}", idx.skipped);
        assert!(idx.skipped[0].contains("bad"), "{:?}", idx.skipped);
        assert!(idx.skipped[0].contains("https"), "{:?}", idx.skipped);
        assert!(idx.skipped[0].contains("entry 2"), "{:?}", idx.skipped);
    }

    #[test]
    fn a_bad_document_is_still_an_error() {
        assert!(Index::parse("not json").is_err());
        assert!(Index::parse(r#"{"packs":[]}"#).is_err());
    }

    #[test]
    fn non_numeric_version_is_rejected_at_parse() {
        let json = r#"{"entries":[
 {"name":"notes","version":"1.2.0","hash":"h1","signer_key":"k","url":"https://example.org/a.tar"},
 {"name":"notes","version":"1.2.0-rc1","hash":"h2","signer_key":"k","url":"https://example.org/b.tar"},
 {"name":"notes","version":"1.x","hash":"h3","signer_key":"k","url":"https://example.org/c.tar"},
 {"name":"notes","version":"","hash":"h4","signer_key":"k","url":"https://example.org/d.tar"},
 {"name":"notes","version":"1..2","hash":"h5","signer_key":"k","url":"https://example.org/e.tar"}
]}"#;
        let idx = Index::parse(json).unwrap();
        assert_eq!(idx.entries.len(), 1);
        assert_eq!(idx.skipped.len(), 4, "{:?}", idx.skipped);
        assert!(idx.skipped.iter().all(|s| s.contains("version")));
        let newest = update_candidate(&idx, ("notes", "h0"), true).unwrap();
        assert_eq!(newest.version, "1.2.0");
        assert!(update_candidate(&idx, ("notes", "h1"), true).is_none());
    }

    #[test]
    fn update_candidate_orders_numerically() {
        let json = r#"{"entries":[
 {"name":"notes","version":"1.9.0","hash":"a","signer_key":"k","url":"https://example.org/a.tar"},
 {"name":"notes","version":"1.10.0","hash":"b","signer_key":"k","url":"https://example.org/b.tar"}
]}"#;
        let idx = Index::parse(json).unwrap();
        let newest = update_candidate(&idx, ("notes", "z"), true).unwrap();
        assert_eq!(newest.version, "1.10.0");
    }

    #[test]
    fn resolve_matches_only_the_pinned_hash() {
        let idx = Index::parse(INDEX).unwrap();
        assert_eq!(resolve(&idx, "notes", "h2").unwrap().version, "1.2.0");
        assert!(resolve(&idx, "notes", "h9").is_err());
        assert!(resolve(&idx, "other", "h2").is_err());
    }

    #[test]
    fn no_update_without_opt_in() {
        let idx = Index::parse(INDEX).unwrap();
        assert!(update_candidate(&idx, ("notes", "h1"), false).is_none());
    }

    #[test]
    fn update_picks_the_newest_unrevoked() {
        let idx = Index::parse(INDEX).unwrap();
        let candidate = update_candidate(&idx, ("notes", "h1"), true).unwrap();
        assert_eq!(candidate.version, "1.2.0");
        assert!(update_candidate(&idx, ("notes", "h2"), true).is_none());
    }

    #[test]
    fn permission_diff_lists_added_and_removed() {
        let idx = Index::parse(INDEX).unwrap();
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
        let idx = Index::parse(INDEX).unwrap();
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
        let idx = Index::parse(INDEX).unwrap();
        let warnings = installed_warnings(&idx, &[("notes", "h4"), ("notes", "h1")]);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("revoked"));
    }
}
