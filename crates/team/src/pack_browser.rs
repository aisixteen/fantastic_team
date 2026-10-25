//! The view-model of the web pack browser (pure MVVM): cards, the selected
//! pack with its permission preview, the opt-in switch, the updates with their
//! permission diff and the warnings for installed revoked packs. The web panel
//! only renders this JSON; it holds no logic. Updates and their diff stay
//! hidden until `opt_in` is true.

use serde_json::{json, Value};

use crate::pack_listing::{list_view, show_view, updates_view, Loaded};

/// What the browser panel shows.
#[derive(Debug, Clone, PartialEq)]
pub struct BrowserVm {
    /// One card per pack.
    pub cards: Vec<Value>,
    /// The selected pack's full view, with its `permission_preview`.
    pub selected: Option<Value>,
    /// Whether the user turned updates on.
    pub opt_in: bool,
    /// Updates with their permission diff; empty until `opt_in`.
    pub updates: Vec<Value>,
    /// Warnings for installed packs that were revoked.
    pub warnings: Vec<String>,
    /// A line for the panel, when something is hidden or wrong.
    pub note: Option<String>,
}

/// Build the browser view-model from what the listing loaded, the installed
/// `(name, version, pinned hash)` triples, the selected pack name and the
/// opt-in switch.
pub fn browser_view(
    loaded: &Loaded,
    pins: &[(String, String, String)],
    selected: Option<&str>,
    opt_in: bool,
) -> BrowserVm {
    let cards = list_view(&loaded.entries, &loaded.installed, None);
    let mut note = None;
    let mut sel = None;
    if let Some(name) = selected.filter(|n| !n.is_empty()) {
        match show_view(&loaded.entries, &loaded.installed, name, None) {
            Ok(mut v) => {
                v["permission_preview"] = json!({"ports": v["ports"], "grants": v["grants"]});
                sel = Some(v);
            }
            Err(e) => note = Some(e),
        }
    }
    let (updates, warnings) = match &loaded.signed {
        Some(index) => updates_view(index, pins, opt_in),
        None => (Vec::new(), Vec::new()),
    };
    if !opt_in && note.is_none() {
        note = Some("turn on updates to see changes".to_string());
    }
    BrowserVm {
        cards,
        selected: sel,
        opt_in,
        updates,
        warnings,
        note,
    }
}

/// The view-model as the JSON the panel renders.
pub fn browser_json(vm: &BrowserVm) -> Value {
    let mut out = json!({
        "cards": vm.cards,
        "opt_in": vm.opt_in,
        "updates": vm.updates,
        "warnings": vm.warnings,
    });
    if let Some(s) = &vm.selected {
        out["selected"] = s.clone();
    }
    if let Some(n) = &vm.note {
        out["note"] = json!(n);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack_listing::{parse_loaded, ListingCfg};
    use crate::pack_registry::{sign_entry, Entry};
    use ed25519_dalek::SigningKey;

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[9u8; 32])
    }

    fn entry(version: &str, hash: &str, grants: &[&str], revoked: bool) -> Value {
        let e = sign_entry(
            &key(),
            Entry {
                name: "notes".into(),
                version: version.into(),
                hash: hash.into(),
                signer_key: String::new(),
                url: "https://example.org/n.json".into(),
                ports: vec!["@connector:github".into()],
                grants: grants.iter().map(|g| g.to_string()).collect(),
                revoked,
                signature: String::new(),
            },
        );
        crate::pack_registry::entry_json(&e)
    }

    fn loaded(entries: Vec<Value>, installed: Vec<(String, String)>) -> Loaded {
        let cfg = ListingCfg {
            index_path: "i".into(),
            templates_dir: "t".into(),
            store: "files".into(),
            trusted_keys: vec![hex::encode(key().verifying_key().to_bytes())],
            registry_url: None,
            allow_insecure: false,
        };
        parse_loaded(&json!({"entries": entries}).to_string(), &cfg, installed).unwrap()
    }

    fn pins() -> Vec<(String, String, String)> {
        vec![("notes".into(), "1.0.0".into(), "h1".into())]
    }

    #[test]
    fn update_diff_hidden_until_opt_in() {
        let l = loaded(
            vec![
                entry("1.0.0", "h1", &[], false),
                entry("1.1.0", "h2", &["repo.write"], false),
            ],
            vec![("notes".into(), "1.0.0".into())],
        );
        let off = browser_view(&l, &pins(), None, false);
        assert!(off.updates.is_empty());
        assert_eq!(off.note.as_deref(), Some("turn on updates to see changes"));
        let on = browser_view(&l, &pins(), None, true);
        assert_eq!(on.updates.len(), 1);
        assert_eq!(
            on.updates[0]["permission_diff"]["added_grants"],
            json!(["repo.write"])
        );
    }

    #[test]
    fn selected_pack_shows_permission_preview() {
        let l = loaded(vec![entry("1.0.0", "h1", &["repo.read"], false)], vec![]);
        let vm = browser_view(&l, &[], Some("notes"), false);
        let s = vm.selected.unwrap();
        assert_eq!(
            s["permission_preview"],
            json!({"ports": ["@connector:github"], "grants": ["repo.read"]})
        );
        let missing = browser_view(&l, &[], Some("nope"), false);
        assert!(missing.selected.is_none());
        assert!(missing.note.unwrap().contains("nope"));
    }

    #[test]
    fn revoked_installed_pack_warns_in_the_browser() {
        let l = loaded(
            vec![entry("1.0.0", "h1", &[], true)],
            vec![("notes".into(), "1.0.0".into())],
        );
        let vm = browser_view(&l, &pins(), None, false);
        assert_eq!(vm.warnings.len(), 1);
        assert!(vm.warnings[0].contains("revoked"));
    }

    #[test]
    fn browser_json_shape_has_cards_updates_warnings() {
        let l = loaded(vec![entry("1.0.0", "h1", &[], false)], vec![]);
        let j = browser_json(&browser_view(&l, &[], None, true));
        assert_eq!(j["cards"].as_array().unwrap().len(), 1);
        assert_eq!(j["updates"], json!([]));
        assert_eq!(j["warnings"], json!([]));
        assert_eq!(j["opt_in"], json!(true));
        assert!(j.get("selected").is_none());
    }
}
