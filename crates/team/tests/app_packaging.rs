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
    for required in ["app", "dmg", "msi", "nsis", "deb", "appimage"] {
        assert!(
            targets.iter().any(|t| t == required),
            "bundle target missing: {required} (targets: {targets:?})"
        );
    }
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
