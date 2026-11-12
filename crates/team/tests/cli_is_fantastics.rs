//! #219: the team binary takes fantastic's command line unchanged. It adds its bundles
//! and its text for `readme`, and nothing else.

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
fn fantastic_s_one_shot_words_run_here() {
    let o = run(&[]);
    assert!(o.status.success(), "{}", text(&o.stderr));
    assert!(text(&o.stdout).contains("fantastic"), "{}", text(&o.stdout));
    // `send` with no kernel in the directory fails loud and starts nothing
    let o = run(&[
        "send",
        r#"{"target":"kernel_state","payload":{"type":"reflect"}}"#,
    ]);
    assert_eq!(o.status.code(), Some(1));
    assert!(text(&o.stderr).contains("no kernel runs on this state folder"));
    // `import` is fantastic's too: it wants a path and a parent
    let o = run(&["import"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(text(&o.stderr).contains("no path"), "{}", text(&o.stderr));
}

/// The launcher passes the team's text to `main_with` as a `readme` section; fantastic
/// owns how sections print.
#[test]
fn the_team_s_text_is_a_section_of_fantastic_s_readme() {
    let o = run(&[]);
    assert!(o.status.success(), "{}", text(&o.stderr));
    let first = fantastic_team::AI_README
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap();
    assert!(text(&o.stdout).contains(first), "{}", text(&o.stdout));
}
