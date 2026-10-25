//! The static listing page: one HTML document with a section per pack. Pure:
//! it takes listing entries and the installed packs and returns text. No
//! script, inline css only. Every value that came from the index passes
//! through [`esc`], so an index cannot inject markup.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::pack_listing::{is_installed, newest, Entry, Measured};

/// Escape text for HTML element content and quoted attributes.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

const NOT_STATED: &str = "not stated";

/// The price as the index states it.
pub fn price_text(price: &Value) -> String {
    match price {
        Value::Null => NOT_STATED.to_string(),
        Value::String(s) if s.trim().is_empty() => NOT_STATED.to_string(),
        Value::String(s) if s.eq_ignore_ascii_case("free") => "free".to_string(),
        Value::String(s) => s.clone(),
        Value::Number(n) if n.as_f64() == Some(0.0) => "free".to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

fn list_html(items: &[String]) -> String {
    if items.is_empty() {
        return "<p class=\"none\">none</p>".to_string();
    }
    let lis: String = items
        .iter()
        .map(|i| format!("<li>{}</li>", esc(i)))
        .collect();
    format!("<ul>{lis}</ul>")
}

fn measured_html(m: &Option<Measured>) -> String {
    let Some(m) = m else {
        return NOT_STATED.to_string();
    };
    let mut parts = Vec::new();
    if let Some(t) = m.tasks {
        parts.push(format!("{t} tasks"));
    }
    if let Some(r) = m.success_rate {
        parts.push(format!("success rate {}%", (r * 100.0).round() as i64));
    }
    if let Some(c) = m.cost_per_task {
        parts.push(format!("cost per task {c}"));
    }
    if parts.is_empty() {
        NOT_STATED.to_string()
    } else {
        esc(&parts.join(", "))
    }
}

fn section(e: &Entry, installed: bool) -> String {
    let mut badges = String::new();
    if installed {
        badges.push_str(" <span class=\"badge installed\">installed</span>");
    }
    if e.revoked {
        badges.push_str(" <span class=\"badge revoked\">revoked</span>");
    }
    let evals = e
        .evals_count
        .map(|n| n.to_string())
        .unwrap_or_else(|| NOT_STATED.to_string());
    format!(
        "<section class=\"pack\"><h2>{name} <small>{version}</small>{badges}</h2>\
<p class=\"summary\">{summary}</p>\
<h3>Ports</h3>{ports}\
<h3>Permissions</h3>{grants}\
<p><b>Evals:</b> {evals}</p>\
<p><b>Measured record:</b> {measured}</p>\
<p><b>Price:</b> {price}</p></section>\n",
        name = esc(&e.name),
        version = esc(&e.version),
        summary = esc(&e.summary),
        ports = list_html(&e.ports),
        grants = list_html(&e.grants),
        evals = esc(&evals),
        measured = measured_html(&e.measured),
        price = esc(&price_text(&e.price)),
    )
}

/// The listing page. One section per pack name, sorted by name, showing the
/// newest unrevoked version (the newest revoked one when all are revoked).
pub fn render_listing(entries: &[Entry], installed: &[(String, String)]) -> String {
    let mut by_name: BTreeMap<&str, Vec<&Entry>> = BTreeMap::new();
    for e in entries {
        by_name.entry(e.name.as_str()).or_default().push(e);
    }
    let body: String = by_name
        .values()
        .filter_map(|versions| newest(versions))
        .map(|e| section(e, is_installed(installed, &e.name)))
        .collect();
    let body = if body.is_empty() {
        "<p class=\"none\">No packs listed.</p>".to_string()
    } else {
        body
    };
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<title>Packs</title><style>\
body{{font:16px/1.5 system-ui,sans-serif;max-width:46rem;margin:0 auto;padding:1rem 16px;background:#fff;color:#111}}\
.pack{{border:1px solid #ccc;border-radius:8px;padding:.5rem 1rem;margin:1rem 0}}\
h2 small{{color:#555;font-weight:normal}}h3{{margin:.6rem 0 0;font-size:1rem}}\
.badge{{font-size:.75rem;border-radius:4px;padding:1px 6px;border:1px solid #888}}\
.revoked{{color:#a00;border-color:#a00}}.installed{{color:#070;border-color:#070}}\
@media (prefers-color-scheme:dark){{body{{background:#111;color:#eee}}.pack{{border-color:#444}}}}\
</style></head><body><h1>Packs</h1>\n{body}</body></html>\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(name: &str, version: &str) -> Entry {
        Entry {
            name: name.into(),
            version: version.into(),
            summary: "does things".into(),
            ports: vec!["@connector:github".into()],
            grants: vec!["repo.read".into()],
            evals_count: Some(3),
            measured: Some(Measured {
                tasks: Some(10),
                success_rate: Some(0.9),
                cost_per_task: Some(0.02),
            }),
            price: json!(5),
            revoked: false,
            signed: None,
        }
    }

    #[test]
    fn page_shows_ports_grants_evals_record_and_price() {
        let html = render_listing(&[entry("notes", "1.0")], &[]);
        for want in [
            "@connector:github",
            "repo.read",
            "Evals:</b> 3",
            "success rate 90%",
            "cost per task 0.02",
            "Price:</b> 5",
            "10 tasks",
        ] {
            assert!(html.contains(want), "missing {want}: {html}");
        }
        let free = Entry {
            price: json!(0),
            ..entry("f", "1.0")
        };
        assert!(render_listing(&[free], &[]).contains("Price:</b> free"));
    }

    #[test]
    fn page_escapes_html_in_every_field() {
        let x = "<script>x</script>";
        let e = Entry {
            name: x.into(),
            version: x.into(),
            summary: x.into(),
            ports: vec![x.into()],
            grants: vec![x.into()],
            price: json!(x),
            ..entry("n", "1")
        };
        let html = render_listing(&[e], &[]);
        assert!(html.contains("&lt;script&gt;x&lt;/script&gt;"));
        assert!(!html.contains("<script"), "{html}");
        assert_eq!(esc("a&b\"c'd"), "a&amp;b&quot;c&#39;d");
    }

    #[test]
    fn page_marks_installed_and_revoked() {
        let mut r = entry("old", "1.0");
        r.revoked = true;
        let html = render_listing(
            &[entry("notes", "1.0"), r],
            &[("notes".into(), "1.0".into())],
        );
        assert_eq!(html.matches("badge installed").count(), 1);
        assert_eq!(html.matches("badge revoked").count(), 1);
    }

    #[test]
    fn page_lists_only_the_newest_unrevoked_version() {
        let mut bad = entry("notes", "3.0");
        bad.revoked = true;
        let html = render_listing(&[entry("notes", "1.0"), entry("notes", "2.0"), bad], &[]);
        assert_eq!(html.matches("<section").count(), 1);
        assert!(html.contains("<small>2.0</small>"));
        assert!(!html.contains("3.0"));
    }

    #[test]
    fn page_shows_not_stated_when_price_and_record_are_absent() {
        let e = Entry {
            price: Value::Null,
            measured: None,
            evals_count: None,
            ..entry("n", "1")
        };
        let html = render_listing(&[e], &[]);
        assert!(html.contains("Price:</b> not stated"));
        assert!(html.contains("Measured record:</b> not stated"));
        assert!(html.contains("Evals:</b> not stated"));
        assert!(render_listing(&[], &[]).contains("No packs listed."));
    }
}
