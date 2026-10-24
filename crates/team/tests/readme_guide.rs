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

#[test]
fn readme_not_yet_no_longer_lists_the_pack_cli_and_mcp() {
    let text = readme();
    let line = text
        .lines()
        .find(|l| l.starts_with("Not yet: signatures"))
        .expect("README must keep its `Not yet` line for the pack verbs");
    assert!(
        !line.contains("CLI, MCP"),
        "the pack CLI and MCP surfaces are built, so `Not yet` must not list them"
    );
    assert!(
        line.contains("the web UI surface for these verbs"),
        "`Not yet` must still list the web UI surface"
    );
}

#[test]
fn readme_describes_the_pack_surfaces() {
    let text = readme();
    assert!(text.contains("fantastic_team packs"), "README must name the CLI");
    assert!(text.contains("`pack_listing`"), "README must name the listing agent");
}

#[test]
fn ai_readme_section_8_names_the_pack_cli() {
    let text = fs::read_to_string(root().join("docs/ai-readme.md")).expect("ai-readme");
    let section = text
        .split("## 8. Pack evals")
        .nth(1)
        .expect("ai-readme has a section 8 for pack evals");
    assert!(
        !section.contains("not a CLI subcommand"),
        "section 8 must not say the verbs have no CLI"
    );
    assert!(section.contains("fantastic_team packs eval"), "section 8 names the CLI form");
    assert!(section.contains("`pack_listing`"), "section 8 names the agent ids");
}
