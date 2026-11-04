//! The team binary: fantastic's command line, unchanged, with this repo's
//! bundles added to the registry (the `extras` of `fantastic_tui::main_with`).
//! The way `canvas` sits on top of the kernel, the team's bundles sit on top of
//! fantastic: no flag of its own, no config file of its own.
//!
//! The binary takes exactly fantastic's words: `--headless`, `canvas [--port N]
//! [--host ADDR]`, `ai <backend> …`, `run …`, `system`, `mcp`, `send`, `import`,
//! `migrate-home` and `readme`. Fantastic runs in the directory it is started in.
//! Three first words are the team's own and handle their arguments themselves:
//! `setup github`, `packs …` and `ai-readme`.

use std::process::ExitCode;

use fantastic_team::register_team;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    // `setup github`: handles its own arguments, then exits.
    if argv.first().map(String::as_str) == Some("setup") {
        return finish(fantastic_team::setup_github::run_cli(&argv[1..]));
    }
    // `packs <verb> [--key value ...]`: send a pack verb to the running team, then
    // stop. Handles its own arguments, like `setup`.
    if argv.first().map(String::as_str) == Some("packs") {
        return finish(fantastic_team::pack_cli::run_cli(&argv[1..]));
    }
    // `ai-readme`: print the README for AIs and stop. No flags, no team.
    if argv.first().map(String::as_str) == Some("ai-readme") {
        print!("{}", fantastic_team::AI_README);
        return ExitCode::SUCCESS;
    }
    // Everything else is fantastic's own command line, parsed by fantastic.
    // The graceful stop can wait on an agent mid-model-call; bound it (#119).
    // The lock registry is keyed by the absolute working directory. The one-shot
    // commands (`send`, `readme`, `import`, …) hold no kernel, so they need none.
    if !fantastic_team::is_one_shot(&argv) {
        if let Ok(cwd) = std::env::current_dir() {
            fantastic_team::watchdog::spawn(cwd, fantastic_team::watchdog::GRACE);
        }
    }
    let extras: &[fantastic_tui::Extra] = &[register_team];
    match fantastic_tui::main_with(argv, extras) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fantastic_team: {e}");
            ExitCode::from(1)
        }
    }
}

fn finish(result: Result<(), String>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fantastic_team: {e}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn ai_readme_subcommand_prints_the_doc_file() {
        // `ai-readme` prints `AI_README`, which is `docs/ai-readme.md` built in.
        let file = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/ai-readme.md"),
        )
        .unwrap();
        assert_eq!(fantastic_team::AI_README, file);
    }

    #[test]
    fn ai_readme_names_fantastic_s_command_line_and_the_removed_flags_only_as_removed() {
        let text = fantastic_team::AI_README;
        for word in [
            "ai-readme",
            "readme",
            "canvas",
            "--port",
            "--headless",
            "import",
            "send",
        ] {
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
