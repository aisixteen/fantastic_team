//! The desktop window (#37): which URL it opens, with what title and size.
//! The spec is pure and always compiled; the Tauri wiring sits behind the
//! off-by-default `tauri` feature. No logic in the wiring.

use crate::validate_loopback;

/// The window title (`app.windows[0].title` in `app/tauri.conf.json`).
pub const TITLE: &str = "Fantastic Team";
/// The window width (`app.windows[0].width`).
pub const WIDTH: f64 = 1280.0;
/// The window height (`app.windows[0].height`).
pub const HEIGHT: f64 = 800.0;

/// What the one window shows.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowSpec {
    pub label: &'static str,
    pub title: String,
    pub width: f64,
    pub height: f64,
    pub url: String,
}

/// The window for `url`. Refuses anything but plain http on loopback, so the
/// window never opens a remote page.
pub fn window_spec(url: &str) -> Result<WindowSpec, String> {
    validate_loopback(url)?;
    Ok(WindowSpec {
        label: "main",
        title: TITLE.to_string(),
        width: WIDTH,
        height: HEIGHT,
        url: url.to_string(),
    })
}

/// Open the window on the main thread and block until the app exits; then
/// call `on_exit`. The packaged config already declares the `main` window,
/// so it is pointed at the spec's URL; without one, it is built here.
#[cfg(feature = "tauri")]
pub fn run_window(spec: WindowSpec, on_exit: impl FnOnce() + Send + 'static) -> Result<(), String> {
    use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

    let url: tauri::Url = spec
        .url
        .parse()
        .map_err(|e| format!("window url {}: {e}", spec.url))?;
    let app = tauri::Builder::default()
        .setup(move |app| {
            if let Some(w) = app.get_webview_window(spec.label) {
                w.set_title(&spec.title)?;
                w.navigate(url)?;
            } else {
                WebviewWindowBuilder::new(app, spec.label, WebviewUrl::External(url))
                    .title(&spec.title)
                    .inner_size(spec.width, spec.height)
                    .build()?;
            }
            Ok(())
        })
        .build(tauri::generate_context!("../../app/tauri.conf.json"))
        .map_err(|e| format!("tauri: {e}"))?;
    let mut on_exit = Some(on_exit);
    app.run(move |_, event| {
        if let RunEvent::Exit = event {
            if let Some(f) = on_exit.take() {
                f();
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{web_url, Target, DEFAULT_PORT};

    #[test]
    fn window_spec_opens_the_launchers_web_url() {
        let url = web_url(&Target::Start {
            binary: "x".into(),
            port: 9123,
        });
        let spec = window_spec(&url).unwrap();
        assert_eq!(spec.url, "http://127.0.0.1:9123/");
        assert_eq!(spec.label, "main");
        assert_eq!(spec.title, TITLE);
    }

    #[test]
    fn window_spec_refuses_a_non_loopback_url() {
        let err = window_spec("http://example.com:8765/").unwrap_err();
        assert!(err.contains("example.com"), "{err}");
        assert!(window_spec("https://127.0.0.1:1/").is_err());
    }

    #[test]
    fn window_spec_accepts_ipv6_and_localhost() {
        assert!(window_spec("http://[::1]:8765/").is_ok());
        assert!(window_spec("http://localhost:8765/").is_ok());
    }

    #[test]
    fn window_spec_matches_tauri_conf() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/tauri.conf.json");
        let text = std::fs::read_to_string(&path).expect("read app/tauri.conf.json");
        let conf: serde_json::Value = serde_json::from_str(&text).expect("conf is JSON");
        let w = &conf["app"]["windows"][0];
        assert_eq!(w["title"].as_str(), Some(TITLE));
        assert_eq!(w["width"].as_f64(), Some(WIDTH));
        assert_eq!(w["height"].as_f64(), Some(HEIGHT));
        let url = web_url(&Target::Start {
            binary: "x".into(),
            port: DEFAULT_PORT,
        });
        let conf_url = w["url"].as_str().expect("window url");
        assert_eq!(url.trim_end_matches('/'), conf_url.trim_end_matches('/'));
    }
}
