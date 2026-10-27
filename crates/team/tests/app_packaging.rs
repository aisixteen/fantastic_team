//! Packaging config checks for the desktop app (`app/tauri.conf.json`).

use std::path::PathBuf;

fn app_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("app")
}

fn load_conf() -> serde_json::Value {
    let path = app_dir().join("tauri.conf.json");
    let text = std::fs::read_to_string(&path).expect("read app/tauri.conf.json");
    serde_json::from_str(&text).expect("tauri.conf.json parses as JSON")
}

#[test]
fn tauri_conf_parses_and_names_the_team_app() {
    let conf = load_conf();
    assert_eq!(conf["productName"], "Fantastic Team");
    assert_eq!(conf["identifier"], "com.aisixteen.team");
}

#[test]
fn bundle_targets_cover_three_desktop_platforms() {
    let conf = load_conf();
    let targets: Vec<String> = conf["bundle"]["targets"]
        .as_array()
        .expect("bundle.targets is an array")
        .iter()
        .map(|t| t.as_str().unwrap_or_default().to_string())
        .collect();
    for required in ["app", "dmg", "msi", "nsis", "deb"] {
        assert!(
            targets.iter().any(|t| t == required),
            "bundle target missing: {required} (targets: {targets:?})"
        );
    }
    assert!(
        !targets.iter().any(|t| t.eq_ignore_ascii_case("appimage")),
        "appimage must not be a target: {targets:?}"
    );
}

#[test]
fn bundle_targets_drop_appimage() {
    let conf = load_conf();
    let targets: Vec<String> = conf["bundle"]["targets"]
        .as_array()
        .expect("bundle.targets is an array")
        .iter()
        .map(|t| t.as_str().unwrap_or_default().to_lowercase())
        .collect();
    assert!(!targets.iter().any(|t| t == "appimage"), "{targets:?}");
    for t in &targets {
        assert!(
            ["app", "dmg", "msi", "nsis", "deb"].contains(&t.as_str()),
            "unexpected bundle target {t:?}"
        );
    }
}

#[test]
fn descriptions_describe_the_agent_tree_app() {
    let conf = load_conf();
    let short = conf["bundle"]["shortDescription"].as_str().unwrap_or("");
    let long = conf["bundle"]["longDescription"].as_str().unwrap_or("");
    assert!(!short.is_empty() && !long.is_empty());
    for d in [short, long] {
        let d = d.to_lowercase();
        assert!(!d.contains("every folder"), "{d}");
        assert!(!d.contains("live canvas for"), "{d}");
    }
    assert!(long.to_lowercase().contains("agent"), "{long}");
}

/// The CSP as directive -> sources.
fn csp() -> std::collections::BTreeMap<String, Vec<String>> {
    let conf = load_conf();
    let text = conf["app"]["security"]["csp"]
        .as_str()
        .expect("a csp string");
    text.split(';')
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(|d| {
            let mut it = d.split_whitespace();
            let name = it.next().unwrap().to_string();
            (name, it.map(str::to_string).collect())
        })
        .collect()
}

/// Every `<name ...>` start tag in `html` as its attribute text (no regex crate).
fn tags<'a>(html: &'a str, name: &str) -> Vec<&'a str> {
    let open = format!("<{name}");
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = html[at..].find(&open) {
        let s = at + i + open.len();
        at = s;
        if !html[s..].starts_with(|c: char| c.is_whitespace() || c == '>') {
            continue;
        }
        let e = s + html[s..].find('>').expect("tag closes");
        out.push(&html[s..e]);
    }
    out
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=\"");
    let i = tag.find(&key)?;
    let v = &tag[i + key.len()..];
    Some(&v[..v.find('"')?])
}

#[test]
fn csp_allows_every_source_the_served_page_uses() {
    let csp = csp();
    let style = csp.get("style-src").expect("style-src");
    assert_eq!(style, &["'self'"]);
    assert!(csp["default-src"] == ["'self'"]);
    for bad in ["unsafe-inline", "unsafe-eval", "data:", "blob:", "https:"] {
        assert!(
            !csp.values().flatten().any(|s| s.contains(bad)),
            "csp allows {bad}"
        );
    }
    let html = fantastic_team::page::index_html();
    assert!(!html.contains("<style"), "inline <style>");
    for t in tags(&html, "script") {
        assert!(attr(t, "src").is_some(), "script without src: {t}");
    }
    let mut at = 0;
    while let Some(i) = html[at..].find("<script") {
        let s = at + i;
        let gt = s + html[s..].find('>').unwrap() + 1;
        let end = gt + html[gt..].find("</script>").expect("script closes");
        assert!(html[gt..end].trim().is_empty(), "inline script body");
        at = end;
    }
    for tag in html.split('<').skip(1) {
        let tag = tag.split('>').next().unwrap();
        assert!(attr(tag, "style").is_none(), "style attribute: {tag}");
        for a in tag.split_whitespace().skip(1) {
            let n = a.split('=').next().unwrap();
            assert!(
                !(n.len() > 2 && n.starts_with("on") && a.contains('=')),
                "handler {a}"
            );
        }
    }
    let files = fantastic_team::page::FILES;
    let mut refs = Vec::new();
    for t in tags(&html, "link") {
        refs.push(attr(t, "href").expect("link href"));
    }
    for t in tags(&html, "script") {
        refs.push(attr(t, "src").unwrap());
    }
    for t in tags(&html, "iframe") {
        refs.push(attr(t, "src").expect("iframe src"));
    }
    assert!(refs.len() >= 3, "{refs:?}");
    for r in refs {
        assert!(
            !r.contains("://") && !r.starts_with("//"),
            "cross-origin {r}"
        );
        let name = r.strip_prefix("/team_page/file/").unwrap_or(r);
        assert!(files.contains(&name), "{r} is not a served file");
    }
}

#[test]
fn served_css_and_js_have_no_csp_violations() {
    let page = fantastic_team::page::file_bytes;
    let text = |f: &str| String::from_utf8(page(f).unwrap()).unwrap();
    for f in ["page.js", "shim.js", "card.js"] {
        let js = text(f);
        for bad in [
            "setAttribute('style'",
            "setAttribute(\"style\"",
            ".cssText",
            "eval(",
            "new Function(",
        ] {
            assert!(!js.contains(bad), "{f} uses {bad}");
        }
    }
    let shim = text("shim.js");
    assert!(shim.contains("location.host"));
    assert!(shim.contains("/team_ws/ws"));
    for lit in ["http://", "https://", "ws://", "wss://"] {
        assert!(!shim.contains(lit), "shim has a {lit} literal");
    }
    let url = fantastic_team::dispatcher::canvas_url("a");
    assert!(url.starts_with('/') || url.starts_with('?'), "{url}");
    let d = fantastic_team::dispatcher::HTML;
    assert!(d.contains("setAttribute('sandbox', 'allow-scripts')"));
    assert!(!d.contains("allow-same-origin"));
    let js = text("page.js");
    assert_eq!(
        js.matches("'script src=\"/team_page/file/card.js\"></'")
            .count(),
        1
    );
    assert!(!text("card.js").contains("<script"));
}

#[test]
fn csp_stays_limited_to_the_one_port() {
    let conf = load_conf();
    let dist = conf["build"]["frontendDist"].as_str().unwrap();
    let port = dist.rsplit(':').next().unwrap();
    let csp = csp();
    assert_eq!(
        csp["connect-src"],
        [
            "'self'".to_string(),
            format!("http://127.0.0.1:{port}"),
            format!("ws://127.0.0.1:{port}")
        ]
    );
    for s in csp.values().flatten() {
        assert!(s != "*" && !s.contains("localhost"), "{s}");
        if s.contains("://") {
            assert!(s.ends_with(&format!("127.0.0.1:{port}")), "other port {s}");
        }
    }
    assert!(!csp.contains_key("script-src"));
    assert!(!csp.contains_key("frame-src"));
}

#[test]
fn every_listed_icon_exists() {
    let conf = load_conf();
    let icons = conf["bundle"]["icon"]
        .as_array()
        .expect("bundle.icon is an array");
    assert!(!icons.is_empty(), "no icons listed");
    for icon in icons {
        let rel = icon.as_str().expect("icon entry is a string");
        let path = app_dir().join(rel);
        assert!(path.is_file(), "listed icon missing: {}", path.display());
    }
}

#[test]
fn no_signing_identity_is_committed() {
    let text = std::fs::read_to_string(app_dir().join("tauri.conf.json")).expect("read conf");
    for key in [
        "signingIdentity",
        "certificateThumbprint",
        "providerShortName",
    ] {
        assert!(!text.contains(key), "signing key committed: {key}");
    }
    for marker in ["APPLE_", "password"] {
        assert!(
            !text.contains(marker),
            "secret-like marker committed: {marker}"
        );
    }
}

#[test]
fn the_window_loads_a_loopback_url() {
    let conf = load_conf();
    let windows = conf["app"]["windows"]
        .as_array()
        .expect("app.windows is an array");
    assert!(!windows.is_empty(), "no window declared");
    for window in windows {
        let url = window["url"].as_str().expect("window url is a string");
        let host_ok = url.starts_with("http://127.0.0.1") || url.starts_with("http://localhost");
        assert!(host_ok, "window url is not loopback: {url}");
    }
}

#[test]
fn window_url_and_frontend_dist_name_the_same_origin() {
    let conf = load_conf();
    let dist = conf["build"]["frontendDist"]
        .as_str()
        .expect("build.frontendDist is a string");
    let url = conf["app"]["windows"][0]["url"]
        .as_str()
        .expect("window url is a string");
    assert_eq!(dist, url, "window url and frontendDist must match");
}

#[test]
fn csp_has_no_unsafe_inline_and_one_port() {
    let conf = load_conf();
    let csp = conf["app"]["security"]["csp"]
        .as_str()
        .expect("app.security.csp is a string");
    assert!(
        !csp.contains("unsafe-inline"),
        "CSP allows unsafe-inline: {csp}"
    );
    assert!(!csp.contains(":*"), "CSP has a wildcard port: {csp}");
    let dist = conf["build"]["frontendDist"]
        .as_str()
        .expect("build.frontendDist is a string");
    let port = dist.rsplit(':').next().expect("frontendDist has a port");
    let expected = format!("connect-src 'self' http://127.0.0.1:{port} ws://127.0.0.1:{port}");
    assert!(
        csp.contains(&expected),
        "connect-src is not limited to the frontendDist port {port}: {csp}"
    );
}

fn team_app_file(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("team_app")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn tauri_feature_is_optional_and_pinned() {
    let toml = team_app_file("Cargo.toml");
    let features = toml
        .split("[features]")
        .nth(1)
        .expect("team_app has a [features] table");
    let features = features.split("\n[").next().unwrap_or_default();
    assert!(
        features
            .lines()
            .any(|l| l.trim_start().starts_with("tauri = [")),
        "no `tauri = [` feature: {features}"
    );
    for line in toml
        .lines()
        .filter(|l| l.trim_start().starts_with("default ="))
    {
        assert!(
            !line.contains("tauri"),
            "tauri is a default feature: {line}"
        );
    }
    let dep = toml
        .lines()
        .find(|l| l.trim_start().starts_with("tauri = {"))
        .expect("a tauri dependency line");
    assert!(
        dep.contains("\"=2.12.1\""),
        "tauri not pinned to =2.12.1: {dep}"
    );
    assert!(dep.contains("optional = true"), "tauri not optional: {dep}");
    let build = toml
        .lines()
        .find(|l| l.trim_start().starts_with("tauri-build = {"))
        .expect("a tauri-build dependency line");
    assert!(
        build.contains("\"=2.7.1\""),
        "tauri-build not pinned to =2.7.1: {build}"
    );
    assert!(
        build.contains("optional = true"),
        "tauri-build not optional: {build}"
    );
}

#[test]
fn team_app_without_feature_has_no_tauri_code_path() {
    let lib = team_app_file("src/lib.rs");
    assert!(
        !lib.contains("tauri::"),
        "lib.rs names tauri:: outside a cfg"
    );
    for rel in ["src/main.rs", "src/window.rs"] {
        let text = team_app_file(rel);
        let marker = text
            .find("#[cfg(feature = \"tauri\")]")
            .unwrap_or_else(|| panic!("{rel} has no #[cfg(feature = \"tauri\")]"));
        for word in ["tauri::", "tauri_build"] {
            if let Some(i) = text.find(word) {
                assert!(i > marker, "{rel} names {word} before the tauri cfg");
            }
        }
    }
}
