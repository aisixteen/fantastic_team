//! The team binary: fantastic's command line, unchanged, with this repo's
//! bundles added to the registry (the `extras` of `fantastic_tui::main_with`).
//! The team's bundles sit on top of fantastic: no config file of its own.
//!
//! The binary takes fantastic's words: `--headless`, `ai <backend> …`, `run …`,
//! `system`, `mcp`, `send`, `import`, `migrate-home` and `readme`. Fantastic runs in the directory it is started in.
//! `fantastic_team readme` prints fantastic's readme followed by this repo's. One recipe word is the team's own, `webui [--port N] [--host ADDR]`: the team page from the binary (`webui.rs`), which passes its flags to fantastic's `--headless --port --host`.

use std::process::ExitCode;

use fantastic_team::register_team;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    // `webui [--port N] [--host ADDR]`: the team page from the binary. Fantastic has no
    // extension point for a recipe word, so the team cuts the word out and hands its
    // flags to fantastic's `--headless --port --host` (`webui.rs`).
    let (argv, page) = match fantastic_team::webui::split(&argv) {
        Ok((None, argv)) => (argv, None),
        Ok((Some(w), rest)) => (webui_argv(&w, rest), Some(w)),
        Err(e) => {
            eprintln!("fantastic_team: {e}");
            return ExitCode::from(2);
        }
    };
    // The launcher seeds the folder before the kernel starts; fantastic never seeds.
    if fantastic_tui::starts_kernel(&argv) {
        if let Err(e) = std::env::current_dir()
            .map_err(|e| e.to_string())
            .and_then(|cwd| fantastic_team::hydrate::seed(&cwd).map_err(|e| e.to_string()))
        {
            eprintln!("fantastic_team: init failed: {e}");
            return ExitCode::from(1);
        }
    }
    // The port is known before the boot only when `--port` was given.
    if let Some(port) = page.as_ref().and_then(|w| w.port) {
        let host = page
            .as_ref()
            .and_then(|w| w.bind.as_deref())
            .unwrap_or(fantastic_web::DEFAULT_HOST);
        println!(
            "the team page: {}",
            fantastic_team::webui::page_url(host, port)
        );
    }
    // Everything else is fantastic's own command line, parsed by fantastic.
    let extras: &[fantastic_tui::Extra] = &[register_team];
    match fantastic_tui::main_with(argv, extras, &[fantastic_team::AI_README]) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fantastic_team: {e}");
            ExitCode::from(1)
        }
    }
}

/// The argv fantastic parses for `webui`: the rest of the command line (`split` put
/// `--headless` in it) with the word's `--port` and `--host`.
fn webui_argv(w: &fantastic_team::webui::WebUi, mut rest: Vec<String>) -> Vec<String> {
    if let Some(port) = w.port {
        rest.extend(["--port".to_string(), port.to_string()]);
    }
    if let Some(bind) = &w.bind {
        rest.extend(["--host".to_string(), bind.clone()]);
    }
    rest
}

#[cfg(test)]
mod tests {
    use super::webui_argv;
    use fantastic_team::webui::WebUi;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn webui_maps_its_flags_to_headless_port_and_host() {
        let w = WebUi {
            port: Some(7000),
            bind: Some("0.0.0.0".into()),
        };
        assert_eq!(
            webui_argv(&w, v(&["--headless"])),
            v(&["--headless", "--port", "7000", "--host", "0.0.0.0"])
        );
    }

    #[test]
    fn webui_without_flags_adds_no_port_or_host() {
        assert_eq!(
            webui_argv(&WebUi::default(), v(&["--headless"])),
            v(&["--headless"])
        );
    }

    #[test]
    fn ai_readme_names_fantastic_s_command_line_and_the_removed_flags_only_as_removed() {
        let text = fantastic_team::AI_README;
        for word in ["readme", "webui", "--port", "--headless", "import", "send"] {
            assert!(text.contains(word), "missing {word}");
        }
        for gone in ["--tui", "--instructions", "--resend-instructions", "--dir"] {
            for line in text.lines().filter(|l| l.contains(gone)) {
                assert!(
                    ["gone", "refuse", "no `--dir`", "unknown"]
                        .iter()
                        .any(|w| line.contains(w)),
                    "the AI readme names the removed {gone} other than as removed: {line}"
                );
            }
        }
    }
}
