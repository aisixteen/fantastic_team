//! #219: the team binary takes fantastic's command line unchanged. It adds its bundles
//! and one first word of its own (`ai-readme`), and nothing
//! else: the flags it used to parse (`--dir`, a bare `--port`, `--tui`, `--instructions`,
//! `--resend-instructions`, `--workers`, `--leads`, `--per-lead`) are fantastic's
//! unknown words now. The old team config file is read nowhere.

use std::path::Path;
use std::process::{Command, Output, Stdio};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_fantastic_team")
}

/// Run the binary in a fresh directory with `args`, stdin closed, and return its output.
fn run(args: &[&str]) -> Output {
    let dir = tempfile::tempdir().unwrap();
    Command::new(bin())
        .args(args)
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn the_flags_the_team_used_to_parse_are_refused_as_fantastic_refuses_them() {
    for args in [
        &["--tui"][..],
        &["--dir", "somewhere"],
        &["--port", "8123"],
        &["--instructions", "do it"],
        &["--instructions-file", "f.md"],
        &["--resend-instructions"],
        &["--workers", "2"],
        &["--leads", "2", "--per-lead", "3"],
        &["--help"],
    ] {
        let o = run(args);
        assert_eq!(
            o.status.code(),
            Some(2),
            "{args:?} must exit 2: {}",
            text(&o.stderr)
        );
        let err = text(&o.stderr);
        // fantastic's own refusal, not a team usage text
        assert!(err.starts_with("fantastic: "), "{args:?}: {err}");
        assert!(!err.contains("fantastic_team ["), "{args:?}: {err}");
    }
}

#[test]
fn a_bare_port_belongs_to_the_canvas_word() {
    let o = run(&["--port", "8123"]);
    assert!(
        text(&o.stderr).contains("--port belongs to the `canvas` recipe"),
        "{}",
        text(&o.stderr)
    );
}

#[test]
fn fantastic_s_recipes_are_parsed_by_fantastic() {
    // each of these is fantastic's own word, refused (or accepted) by fantastic's parser
    let cases: [(&[&str], &str); 8] = [
        (&["canvas", "--headless"], "`canvas` runs headless already"),
        (&["canvas", "--port", "0"], "--port must be 1..65535"),
        (
            &["canvas", "--host", "localhost"],
            "--host must be an IP address",
        ),
        (&["ai", "vllm"], "`ai vllm` needs -p"),
        (&["ai", "anthropic"], "`ai anthropic` needs --model"),
        (
            &["ai", "bogus"],
            "`ai` takes vllm, ollama, responses or anthropic",
        ),
        (&["run"], "`run` needs the command to run"),
        (&["--headless", "run", "htop"], "`run` needs a View"),
    ];
    for (args, want) in cases {
        let o = run(args);
        assert_eq!(o.status.code(), Some(2), "{args:?}");
        assert!(
            text(&o.stderr).contains(want),
            "{args:?}: {}",
            text(&o.stderr)
        );
    }
}

#[test]
fn fantastic_s_one_shot_words_run_here() {
    let o = run(&["readme"]);
    assert!(o.status.success(), "{}", text(&o.stderr));
    assert!(text(&o.stdout).contains("fantastic"), "{}", text(&o.stdout));
    // `send` with no kernel in the directory fails loud and starts nothing
    let o = run(&[
        "send",
        r#"{"target":"kernel_state","payload":{"type":"reflect"}}"#,
    ]);
    assert_eq!(o.status.code(), Some(1));
    assert!(text(&o.stderr).contains("no kernel runs in this directory"));
    // `import` is fantastic's too: it wants a path and a parent
    let o = run(&["import"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(text(&o.stderr).contains("no path"), "{}", text(&o.stderr));
}

#[test]
fn the_team_s_own_first_words_are_still_the_teams() {
    let o = run(&["ai-readme"]);
    assert!(o.status.success());
    assert_eq!(text(&o.stdout), fantastic_team::AI_README);
}

#[cfg(unix)]
mod running {
    use super::*;
    use std::time::{Duration, Instant};

    fn send(dir: &Path, item: &str) -> Output {
        Command::new(bin())
            .args(["send", item])
            .current_dir(dir)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn wait_for_kernel(dir: &Path, child: &mut std::process::Child) {
        let end = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(st) = child.try_wait().unwrap() {
                panic!("the team exited early: {st}");
            }
            if send(
                dir,
                r#"{"target":"kernel_state","payload":{"type":"reflect"}}"#,
            )
            .status
            .success()
            {
                return;
            }
            assert!(Instant::now() < end, "no kernel answered `send`");
            std::thread::sleep(Duration::from_millis(300));
        }
    }

    fn stop(child: &mut std::process::Child) {
        let _ = Command::new("kill").arg(child.id().to_string()).status();
        let end = Instant::now() + Duration::from_secs(10);
        while child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < end, "the team did not stop on SIGTERM");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// `--headless` is no screen and no port; the canvas is registered but not started,
    /// and an agent can still make a canvas with `create_agent`, with no `canvas` word.
    #[test]
    fn headless_starts_no_canvas_and_an_agent_can_create_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = Command::new(bin())
            .arg("--headless")
            .current_dir(dir.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        wait_for_kernel(dir.path(), &mut child);
        let listed = send(
            dir.path(),
            r#"{"target":"kernel_state","payload":{"type":"list_agents"}}"#,
        );
        let listed = text(&listed.stdout);
        for absent in ["canvas_page", "web", "canvas_ws"] {
            assert!(
                !listed.contains(&format!("\"{absent}\"")),
                "`{absent}` was started without the `canvas` word: {listed}"
            );
        }
        let made = send(
            dir.path(),
            r#"{"target":"kernel_state","payload":{"type":"create_agent","parent_id":"kernel_state","id":"a_canvas","handler_module":"canvas_page.tools"}}"#,
        );
        assert!(
            made.status.success(),
            "create_agent of a canvas: {} {}",
            text(&made.stdout),
            text(&made.stderr)
        );
        stop(&mut child);
        let out = {
            let mut s = String::new();
            use std::io::Read;
            child.stdout.take().unwrap().read_to_string(&mut s).unwrap();
            s
        };
        assert!(
            !out.contains("the canvas:"),
            "headless served a canvas: {out}"
        );
    }
}
