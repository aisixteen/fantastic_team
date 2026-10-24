//! The README is a guide a third party follows. These tests keep it honest:
//! its sections are in order, the scripts it names exist, the config keys in
//! its examples are read by the code, and its examples hold no secret values.

use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn readme() -> String {
    fs::read_to_string(root().join("README.md")).expect("README.md must exist")
}

#[test]
fn readme_has_the_guide_sections_in_order() {
    let text = readme();
    let lines: Vec<&str> = text.lines().map(|l| l.trim()).collect();
    let wanted = [
        "## Install",
        "## Configure",
        "## Lanes",
        "## Run",
        "## Observe",
        "## Security model",
    ];
    let mut at = 0usize;
    for h in wanted {
        let pos = lines
            .iter()
            .position(|l| *l == h)
            .unwrap_or_else(|| panic!("README is missing the header `{h}`"));
        assert!(pos >= at, "README header `{h}` is out of order");
        at = pos;
    }
}

#[test]
fn every_script_the_readme_names_exists() {
    let text = readme();
    let mut found = 0;
    let mut rest = text.as_str();
    while let Some(i) = rest.find("scripts/") {
        rest = &rest[i + "scripts/".len()..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
            .collect();
        if name.ends_with(".sh") {
            found += 1;
            let path = root().join("scripts").join(&name);
            assert!(
                path.exists(),
                "README names scripts/{name}, which does not exist"
            );
        }
    }
    assert!(found > 0, "README should name at least one script");
}

#[test]
fn every_config_key_the_readme_names_is_read_by_the_code() {
    let text = readme();
    let code = format!(
        "{}\n{}",
        fs::read_to_string(root().join("crates/team/src/config.rs")).unwrap(),
        fs::read_to_string(root().join("crates/team/src/repo_config.rs")).unwrap(),
    );
    let mut in_yaml = false;
    let mut checked = 0;
    let mut parent = String::new();
    for raw in text.lines() {
        let line = raw.trim_start();
        if line.starts_with("```") {
            in_yaml = line == "```yaml";
            parent.clear();
            continue;
        }
        if !in_yaml {
            continue;
        }
        let indent = raw.len() - line.len();
        if indent == 0 {
            parent = line.trim_end_matches(':').to_string();
        } else if indent == 2 && (parent == "templates" || parent == "connectors") {
            // a user-chosen name (template or connector id), not a config key
            continue;
        }
        let body = line.trim_start_matches("- ").trim_start();
        let key: String = body
            .chars()
            .take_while(|c| c.is_ascii_lowercase() || *c == '_')
            .collect();
        if !key.is_empty() && body[key.len()..].starts_with(':') {
            checked += 1;
            assert!(
                code.contains(&key),
                "README example uses key `{key}`, which config.rs / repo_config.rs do not read"
            );
        }
    }
    assert!(checked > 0, "README should have yaml config examples");
}

#[test]
fn readme_examples_hold_no_secret_values() {
    let text = readme();
    for bad in ["ghp_", "-----BEGIN", "sk-"] {
        assert!(!text.contains(bad), "README must not contain `{bad}`");
    }
}

/// The fenced ```yaml blocks of the README, without their fences.
fn readme_yaml_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        match current.as_mut() {
            None => {
                if line.trim_end() == "```yaml" {
                    current = Some(String::new());
                }
            }
            Some(body) => {
                if line.trim_end() == "```" {
                    blocks.push(current.take().unwrap());
                } else {
                    body.push_str(line);
                    body.push('\n');
                }
            }
        }
    }
    blocks
}

#[test]
fn readme_defaults_match_repo_config() {
    let text = readme();
    let defaults = text
        .lines()
        .find(|l| l.starts_with("Every key is optional"))
        .expect("README must state the defaults in an `Every key is optional` line");
    // The code's own answer for an empty file, not a grep of its source.
    let cfg = fantastic_team::repo_config::parse("").expect("an empty team.yaml must parse");
    let label = fantastic_team::repo_config::DEFAULT_LABEL;
    assert_eq!(cfg.task_labels, vec![label.to_string()]);
    assert_eq!(cfg.review.approvals, 1);
    assert!(
        cfg.review.approvers.is_empty(),
        "default approvers is any reviewer"
    );
    assert!(
        defaults.contains(&format!("`{label}`")),
        "README default label disagrees with DEFAULT_LABEL `{label}`"
    );
    assert!(
        defaults.contains("one approval"),
        "README must say the default is one approval"
    );
}

#[test]
fn readme_repo_yaml_example_parses() {
    let text = readme();
    let block = readme_yaml_blocks(&text)
        .into_iter()
        .find(|b| b.lines().any(|l| l.starts_with("task_labels:")))
        .expect("README must show a team.yaml example with task_labels");
    let cfg = fantastic_team::repo_config::parse(&block)
        .unwrap_or_else(|e| panic!("README team.yaml example must parse: {e}"));
    assert_eq!(cfg.task_labels, vec!["team-ready".to_string()]);
    assert_eq!(
        cfg.paths,
        vec!["src/**".to_string(), "docs/*.md".to_string()]
    );
    assert_eq!(cfg.build.as_deref(), Some("cargo build"));
    assert_eq!(cfg.test.as_deref(), Some("cargo test"));
    assert_eq!(cfg.review.approvals, 1);
    assert_eq!(cfg.review.approvers, vec!["alice".to_string()]);
}

/// Every `.rs` file under `dir` outside a `tests` folder, concatenated.
fn rust_sources(dir: &std::path::Path) -> String {
    let mut out = String::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).expect("source dir must be readable") {
            let p = entry.unwrap().path();
            if p.is_dir() {
                // the tests name the phrases they check; only the code counts
                if p.file_name().is_none_or(|n| n != "tests") {
                    stack.push(p);
                }
            } else if p.extension().is_some_and(|e| e == "rs") {
                out.push_str(&fs::read_to_string(&p).unwrap());
                out.push('\n');
            }
        }
    }
    out
}

#[test]
fn readme_names_every_cli_subcommand_and_flag() {
    let text = readme();
    let source = format!(
        "{}\n{}\n{}",
        fs::read_to_string(root().join("crates/team/src/main.rs")).unwrap(),
        fs::read_to_string(root().join("crates/team/src/instructions.rs")).unwrap(),
        fs::read_to_string(root().join("crates/team/src/setup_github.rs")).unwrap(),
    );
    for name in [
        "setup github",
        "ai-readme",
        "--instructions-file",
        "--resend-instructions",
        "--per-lead",
    ] {
        assert!(
            text.contains(name),
            "README must document `{name}` (it is in the binary)"
        );
        assert!(source.contains(name), "`{name}` is not in the code");
    }
}

#[test]
fn readme_documents_the_e2e_tiers_and_manual_ci() {
    let text = readme();
    for want in ["e2e/run.py", "--only-free", "--only-turns", "manual"] {
        assert!(text.contains(want), "README must name `{want}`");
    }
    // every `python3 e2e/...` path the README names exists on disk
    let mut checked = 0;
    for line in text.lines() {
        if let Some(i) = line.find("python3 e2e/") {
            let rest = &line[i + "python3 ".len()..];
            let path: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || "/._-".contains(*c))
                .collect();
            checked += 1;
            assert!(
                root().join(&path).exists(),
                "README names `{path}`, which does not exist"
            );
        }
    }
    assert!(checked > 0, "README should name an e2e command");
}

#[test]
fn readme_team_yaml_optional_keys_are_documented() {
    let text = readme();
    let config = fs::read_to_string(root().join("crates/team/src/config.rs")).unwrap();
    for (key, field) in [("groups:", "pub groups"), ("board:", "pub board")] {
        assert!(text.contains(key), "README must document the `{key}` key");
        assert!(
            config.contains(field),
            "`{key}` is not read by config.rs (`{field}`)"
        );
    }
    assert!(
        text.contains("examples/team.yaml") && text.contains("templates/"),
        "README must point to examples/team.yaml and templates/"
    );
}

#[test]
fn readme_does_not_name_unimplemented_features() {
    let text = readme();
    let code = format!(
        "{}{}",
        rust_sources(&root().join("crates")),
        rust_sources(&root().join("app"))
    );
    for phrase in ["app-merge", "fewest tasks"] {
        if text.contains(phrase) {
            assert!(
                code.contains(phrase),
                "README claims `{phrase}`, which no code under crates/ or app/ implements"
            );
        }
    }
}

#[test]
fn readme_mentions_the_app() {
    let text = readme();
    assert!(
        text.contains("app/README.md"),
        "README must link app/README.md"
    );
    assert!(root().join("app/README.md").exists());
    assert!(root().join("app/tauri.conf.json").exists());
}

#[test]
fn install_guide_names_the_setup_command() {
    let install = fs::read_to_string(root().join("docs/install.md")).unwrap();
    assert!(
        install.contains("fantastic_team setup github"),
        "docs/install.md must name `fantastic_team setup github`"
    );
    assert!(
        !install.contains("manifest flow is not in the repo"),
        "docs/install.md still says the manifest flow is not in the repo"
    );
}
