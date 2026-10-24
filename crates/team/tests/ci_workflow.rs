//! Checks on the manual CI workflow (`.github/workflows/ci.yml`, #46). CI runs by
//! hand only: the trigger is `workflow_dispatch` and nothing else.

use std::path::PathBuf;

use serde_yaml::Value;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn raw() -> String {
    let path = repo_root().join(".github").join("workflows").join("ci.yml");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn doc() -> Value {
    serde_yaml::from_str(&raw()).expect("ci.yml parses as YAML")
}

/// YAML 1.1 reads a bare `on:` key as the boolean true; serde_yaml 0.9 reads it as
/// the string "on". Accept both.
fn key<'a>(map: &'a Value, name: &str) -> Option<&'a Value> {
    let m = map.as_mapping()?;
    m.get(Value::String(name.to_string())).or_else(|| {
        if name == "on" {
            m.get(Value::Bool(true))
        } else {
            None
        }
    })
}

fn triggers(doc: &Value) -> &Value {
    key(doc, "on").expect("ci.yml has an `on:` block")
}

fn job<'a>(doc: &'a Value, name: &str) -> &'a Value {
    key(key(doc, "jobs").expect("ci.yml has `jobs:`"), name)
        .unwrap_or_else(|| panic!("job `{name}` missing"))
}

fn needs(job: &Value) -> Vec<String> {
    match key(job, "needs") {
        None => Vec::new(),
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Sequence(seq)) => seq
            .iter()
            .map(|v| v.as_str().unwrap_or_default().to_string())
            .collect(),
        Some(other) => panic!("unexpected needs shape: {other:?}"),
    }
}

fn if_text(job: &Value) -> String {
    key(job, "if")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn run_text(job: &Value) -> String {
    let steps = key(job, "steps")
        .and_then(Value::as_sequence)
        .expect("job has steps");
    steps
        .iter()
        .filter_map(|s| key(s, "run").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn ci_workflow_parses_and_is_dispatch_only() {
    let doc = doc();
    let on = triggers(&doc).as_mapping().expect("`on:` is a mapping");
    let keys: Vec<String> = on
        .keys()
        .map(|k| k.as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        keys,
        vec!["workflow_dispatch".to_string()],
        "triggers: {keys:?}"
    );
    for banned in [
        "push",
        "pull_request",
        "pull_request_target",
        "schedule",
        "release",
        "tags",
        "workflow_call",
    ] {
        assert!(
            !keys.iter().any(|k| k == banned),
            "trigger {banned} must not be set"
        );
    }
}

#[test]
fn ci_workflow_has_e2e_boolean_input_default_false() {
    let doc = doc();
    let e2e = key(
        key(
            key(triggers(&doc), "workflow_dispatch").expect("workflow_dispatch"),
            "inputs",
        )
        .expect("workflow_dispatch has inputs"),
        "e2e",
    )
    .expect("input `e2e` exists");
    assert_eq!(key(e2e, "type").and_then(Value::as_str), Some("boolean"));
    assert_eq!(key(e2e, "default"), Some(&Value::Bool(false)));
}

#[test]
fn ci_units_cover_three_oses_and_four_checks() {
    let doc = doc();
    let units = job(&doc, "units");
    let os: Vec<String> = key(
        key(key(units, "strategy").unwrap(), "matrix").unwrap(),
        "os",
    )
    .and_then(Value::as_sequence)
    .expect("units matrix os is a list")
    .iter()
    .map(|v| v.as_str().unwrap_or_default().to_string())
    .collect();
    for want in ["ubuntu-latest", "macos-latest", "windows-latest"] {
        assert!(
            os.iter().any(|o| o == want),
            "units matrix lacks {want}: {os:?}"
        );
    }
    let run = run_text(units);
    for want in [
        "cargo fmt --check",
        "cargo clippy --workspace --all-targets",
        "-D warnings",
        "cargo test --workspace",
        "scripts/check-secrets.sh",
    ] {
        assert!(run.contains(want), "units steps lack `{want}`");
    }
}

#[test]
fn ci_full_jobs_gated_on_e2e_and_units() {
    let doc = doc();
    let build = job(&doc, "build");
    assert!(
        if_text(build).contains("inputs.e2e"),
        "build job must be gated on inputs.e2e"
    );
    assert_eq!(needs(build), vec!["units".to_string()], "build needs units");

    let e2e = job(&doc, "e2e");
    assert!(
        if_text(e2e).contains("inputs.e2e"),
        "e2e job must be gated on inputs.e2e"
    );
    assert_eq!(needs(e2e), vec!["build".to_string()], "e2e needs build");

    let units = job(&doc, "units");
    assert!(
        key(units, "if").is_none(),
        "units job must run without an `if`"
    );
}

#[test]
fn ci_has_no_secrets_and_cancels_older_runs() {
    // The guard script is named check-secrets.sh; that name is not a secrets context.
    let text = raw().replace("check-secrets.sh", "");
    for banned in ["secrets.", "GH_TOKEN", "API_KEY"] {
        assert!(!text.contains(banned), "ci.yml mentions {banned}");
    }
    let doc = doc();
    let concurrency = key(&doc, "concurrency").expect("concurrency block");
    assert_eq!(
        key(concurrency, "cancel-in-progress"),
        Some(&Value::Bool(true)),
        "a newer run must cancel the older one"
    );
    let title = key(&doc, "run-name")
        .and_then(Value::as_str)
        .expect("run-name is set");
    assert!(
        title.contains("units") && title.contains("full"),
        "run-name: {title}"
    );
}

#[test]
fn ci_e2e_runs_the_driver_on_built_binaries() {
    let doc = doc();
    let e2e = job(&doc, "e2e");
    assert!(
        run_text(e2e).contains("tests/e2e.sh"),
        "e2e job must run tests/e2e.sh"
    );
    assert!(
        repo_root().join("tests").join("e2e.sh").is_file(),
        "tests/e2e.sh missing"
    );
}

#[test]
fn ci_builds_and_tests_both_builds() {
    let doc = doc();
    let root = repo_root();
    assert!(root.join("patches").join("github-exec.patch").is_file());
    for script in ["build.sh", "build-github.sh"] {
        assert!(root.join("scripts").join(script).is_file(), "{script}");
    }
    assert!(root.join("tests").join("build_github.sh").is_file());
    let units = run_text(job(&doc, "units"));
    assert!(
        units.contains("scripts/build-github.sh --check"),
        "units lack the patch check"
    );
    assert!(
        units.contains("scripts/build-github.sh --test"),
        "units lack the github tests"
    );
    assert!(
        units.contains("tests/build_github.sh"),
        "units lack the build script test"
    );
    let build = run_text(job(&doc, "build"));
    assert!(
        build.contains("scripts/build-github.sh"),
        "build job lacks the github build"
    );
    let text = raw();
    assert!(text.contains("name: fantastic_team-${{ matrix.os }}"));
    assert!(text.contains("name: fantastic_team-github-${{ matrix.os }}"));
    let e2e = run_text(job(&doc, "e2e"));
    assert!(
        e2e.contains("--build general") && e2e.contains("--build github"),
        "{e2e}"
    );
}

#[test]
fn repro_job_builds_twice_in_different_dirs_and_has_no_signing() {
    let doc = doc();
    let repro = job(&doc, "repro");
    assert!(
        if_text(repro).contains("inputs.e2e"),
        "repro job must be gated on inputs.e2e"
    );
    assert_eq!(needs(repro), vec!["units".to_string()], "repro needs units");
    let text = run_text(repro);
    for want in ["scripts/build.sh", "scripts/build-github.sh", "SHA256SUMS"] {
        assert!(text.contains(want), "repro steps lack `{want}`");
    }
    assert!(
        text.contains("RUNNER_TEMP") && text.contains("CARGO_TARGET_DIR"),
        "repro must build in two directories with two target dirs"
    );
    // Signing stays on the maintainer's laptop: nothing in the job reaches for secrets or a signer.
    let whole = serde_yaml::to_string(repro).expect("repro job serializes");
    for banned in ["secrets.", "gpg", "codesign", "notarytool"] {
        assert!(
            !whole.to_lowercase().contains(banned),
            "repro job mentions {banned}"
        );
    }
    assert!(
        !raw().contains("@stable"),
        "ci.yml must take the toolchain from rust-toolchain.toml, not @stable"
    );
    assert!(
        run_text(job(&doc, "units")).contains("tests/repro_scripts.sh"),
        "units shell tests must run tests/repro_scripts.sh"
    );
    let inputs = key(
        key(triggers(&doc), "workflow_dispatch").expect("workflow_dispatch"),
        "inputs",
    )
    .expect("inputs");
    let strict = key(inputs, "strict").expect("input `strict` exists");
    assert_eq!(key(strict, "type").and_then(Value::as_str), Some("boolean"));
    assert_eq!(key(strict, "default"), Some(&Value::Bool(false)));

    let pin = std::fs::read_to_string(repo_root().join("rust-toolchain.toml"))
        .expect("rust-toolchain.toml exists");
    assert!(pin.contains("channel = \"1."), "toolchain pin: {pin}");
    // Every job that builds or tests guards that rustc is the pin, not whatever the runner has.
    for name in ["units", "build", "repro"] {
        assert!(
            run_text(job(&doc, name)).contains("rustc -V"),
            "job `{name}` lacks the rustc -V guard against the pin"
        );
    }
}
