//! The README is a guide a third party follows. These tests keep it honest:
//! its sections are in order, the scripts it names exist, the record fields in
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

/// The README's yaml blocks that are record fields (not a whole blueprint), by their first key.
fn record_block(text: &str, first_key: &str) -> serde_json::Value {
    let block = readme_yaml_blocks(text)
        .into_iter()
        .find(|b| b.lines().next().is_some_and(|l| l.starts_with(first_key)))
        .unwrap_or_else(|| panic!("README must show a yaml block starting with `{first_key}`"));
    serde_yaml::from_str(&block).unwrap_or_else(|e| panic!("`{first_key}` block is YAML: {e}"))
}

#[test]
fn every_record_field_the_readme_names_is_read_by_the_code() {
    let text = readme();
    // the dispatcher's `groups`, the board's `team_groups`, `repos` and `github`
    let code = rust_sources(&root().join("crates/team/src"));
    let groups = record_block(&text, "groups:");
    for key in groups.as_object().unwrap().keys() {
        assert!(
            code.contains(&format!("\"{key}\"")),
            "README example uses the record field `{key}`, which the code does not read"
        );
    }
    // the roles registry's own fields: a registry made from the README block, in a team
    // whose kernel holds the stock roles (embedded), lists the six roles and holds the limit
    let reg = record_block(&text, "limits:");
    let dir = tempfile::TempDir::new().unwrap();
    let mut record = reg.as_object().unwrap().clone();
    record.insert("type".into(), "create_agent".into());
    record.insert("id".into(), "roles".into());
    record.insert("parent_id".into(), "kernel".into());
    record.insert("handler_module".into(), "agentic_resources.tools".into());
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        use fantastic_kernel::Item;
        let kernel = fantastic_team::start(dir.path()).await.unwrap();
        let made = kernel
            .send(vec![Item::new("kernel", serde_json::Value::Object(record))])
            .await;
        assert!(made[0].result.get("error").is_none(), "{}", made[0].result);
        let roles = kernel
            .send(vec![Item::new(
                "roles",
                serde_json::json!({"type": "roles"}),
            )])
            .await;
        let names = roles[0].result.to_string();
        for card in ["worker", "lead", "project_lead", "issue_keeper"] {
            assert!(names.contains(card), "{card} not listed: {names}");
        }
        let count = kernel
            .send(vec![Item::new(
                "roles",
                serde_json::json!({"type": "get_count"}),
            )])
            .await;
        assert!(
            count[0].result.to_string().contains('3'),
            "the README's total limit is not read: {}",
            count[0].result
        );
        kernel.shutdown().await;
        fantastic_kernel::bootstrap::shutdown(dir.path()).unwrap();
    });
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
    let cfg =
        fantastic_team::repo_config::from_record(None).expect("no entry must give the defaults");
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
        .expect("README must show a repo_config example with task_labels");
    let entry: serde_json::Value = serde_yaml::from_str(&block).expect("the example is YAML");
    let cfg = fantastic_team::repo_config::from_record(Some(&entry))
        .unwrap_or_else(|e| panic!("README repo_config example must parse: {e}"));
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
    let source = fs::read_to_string(root().join("crates/team/src/main.rs")).unwrap();
    // the team's own first words are in the binary and in the README
    for name in ["ai-readme"] {
        assert!(
            text.contains(name),
            "README must document `{name}` (it is in the binary)"
        );
        assert!(source.contains(name), "`{name}` is not in the code");
    }
    // everything else is fantastic's command line, which the README says it is
    assert!(
        text.contains("The command line is fantastic's, unchanged"),
        "README must say the CLI is fantastic's"
    );
    for word in [
        "canvas",
        "--headless",
        "ai ",
        "run ",
        "system",
        "mcp",
        "send",
        "import",
        "readme",
    ] {
        assert!(text.contains(word), "README must name fantastic's `{word}`");
    }
    // the removed flags are named only as removed
    for gone in [
        "--tui",
        "--instructions",
        "--resend-instructions",
        "--per-lead",
    ] {
        for line in text.lines().filter(|l| l.contains(gone)) {
            assert!(
                line.contains("refused") || line.contains("no longer"),
                "README mentions the removed `{gone}` other than as removed: {line}"
            );
        }
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
fn readme_record_fields_are_the_ones_the_code_reads() {
    let text = readme();
    // each field the README sends a former config key to is read by the code
    let sources = rust_sources(&root().join("crates/team/src"));
    for (field, needle) in [
        ("tiers", "\"tiers\""),
        ("team_groups", "\"team_groups\""),
        ("group_settings", "\"group_settings\""),
        ("repo_config", "\"repo_config\""),
        ("groups", "\"groups\""),
    ] {
        assert!(
            text.contains(&format!("`{field}`")),
            "README must document the `{field}` field"
        );
        assert!(
            sources.contains(needle),
            "`{field}` is not read by the code (`{needle}`)"
        );
    }
    assert!(
        text.contains("examples/create_agent/worker_only.sh")
            && text.contains("crates/team/src/new_team.rs")
            && text.contains("templates/"),
        "README must point to the create_agent example, new_team.rs and templates/"
    );
    assert!(root()
        .join("examples/create_agent/worker_only.sh")
        .is_file());
    assert!(root().join("crates/team/src/new_team.rs").is_file());
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
fn install_guide_names_the_gh_token_env_ref() {
    let install = fs::read_to_string(root().join("docs/install.md")).unwrap();
    assert!(
        install.contains("GH_TOKEN"),
        "docs/install.md must name GH_TOKEN"
    );
    assert!(
        install.contains("local only"),
        "docs/install.md must say what no credential means"
    );
}

/// The shell test scripts CI's units job runs: the `sh tests/*.sh` lines of ci.yml.
fn ci_shell_tests() -> Vec<String> {
    let ci = fs::read_to_string(root().join(".github/workflows/ci.yml")).expect("ci.yml");
    ci.lines()
        .map(|l| l.trim())
        .filter(|l| l.starts_with("sh tests/") && l.ends_with(".sh"))
        .map(|l| l["sh ".len()..].to_string())
        .collect()
}

#[test]
fn readme_lists_every_shell_test_ci_runs() {
    let text = readme();
    let tests = ci_shell_tests();
    assert!(
        tests.len() >= 4,
        "ci.yml should run at least 4 shell tests, found {}: {tests:?}",
        tests.len()
    );
    for path in &tests {
        assert!(
            text.contains(path.as_str()),
            "README does not list `{path}`, which ci.yml runs"
        );
    }
}

#[test]
fn readme_names_the_github_build_check_and_test_steps() {
    let ci = fs::read_to_string(root().join(".github/workflows/ci.yml")).expect("ci.yml");
    let text = readme();
    for step in ["build-github.sh --check", "build-github.sh --test"] {
        if ci.contains(step) {
            assert!(
                text.contains(step),
                "ci.yml runs `{step}`, which the README does not name"
            );
        }
    }
}

#[test]
fn install_doc_says_the_cli_is_fantastics() {
    let install = fs::read_to_string(root().join("docs/install.md")).unwrap();
    assert!(
        install.contains("The command line is fantastic's, unchanged"),
        "docs/install.md must say the CLI is fantastic's"
    );
}

#[test]
fn readme_create_agent_example_is_the_worker_only_script() {
    let text = readme();
    let start = text
        .find("```json\n{\"type\": \"create_agent\", \"id\": \"w1\"")
        .expect("README must show the worker-only create_agent payload");
    let body = &text[start + "```json\n".len()..];
    let shown: serde_json::Value =
        serde_json::from_str(body[..body.find("\n```").unwrap()].trim()).expect("README JSON");
    let script = fs::read_to_string(root().join("examples/create_agent/worker_only.sh")).unwrap();
    let on_disk = script
        .lines()
        .filter_map(|l| {
            l.strip_prefix("$T send '")
                .and_then(|r| r.strip_suffix('\''))
        })
        .map(|j| serde_json::from_str::<serde_json::Value>(j).expect("script item is JSON"))
        .find(|i| i["payload"]["type"] == "create_agent")
        .expect("the script creates the worker");
    assert_eq!(
        shown, on_disk["payload"],
        "README example drifted from the script"
    );
}

/// The example script's items, run in order on a seeded home with this repo's bundles:
/// the worker exists with its fields, and both ends of its edges are written.
#[tokio::test(flavor = "multi_thread")]
async fn the_create_agent_script_makes_its_worker_on_a_seeded_home() {
    use fantastic_kernel::{AgentId, Item};
    let script = fs::read_to_string(root().join("examples/create_agent/worker_only.sh")).unwrap();
    let items: Vec<serde_json::Value> = script
        .lines()
        .filter_map(|l| {
            l.strip_prefix("$T send '")
                .and_then(|r| r.strip_suffix('\''))
        })
        .map(|j| serde_json::from_str(j).unwrap())
        .collect();
    assert_eq!(items.len(), 4, "list, create, two grants");
    let dir = tempfile::TempDir::new().unwrap();
    fantastic_tui::seed_dir(dir.path()).unwrap();
    let kernel = fantastic_team::start(dir.path()).await.unwrap();
    let mk = serde_json::json!({"type": "create_agent", "parent_id": "user", "id": "credentials",
        "handler_module": "credential_store.tools", "backend": "env", "refs": {}});
    let o = kernel
        .send(vec![Item::new(fantastic_team::tree::ROOT_ID, mk)])
        .await;
    assert!(o[0].result.get("error").is_none(), "{}", o[0].result);
    for item in &items {
        let out = kernel
            .send(vec![Item::new(
                item["target"].as_str().unwrap(),
                item["payload"].clone(),
            )])
            .await;
        assert!(
            out[0].result.get("error").is_none(),
            "{item}: {}",
            out[0].result
        );
    }
    let meta = |id: &str| {
        kernel
            .agents
            .get(&AgentId::from(id))
            .map(|a| a.meta.read().unwrap().clone())
            .unwrap()
    };
    let w = meta("w1");
    assert_eq!(w["model"], "example-model");
    assert_eq!(w["memory_store_id"], "memory");
    assert_eq!(w["credentials"]["store"], "credentials");
    assert_eq!(meta("memory")["callable_by"]["w1"], "*");
    assert_eq!(
        meta("memory")["callable_by"]["user"],
        "*",
        "what it had stays"
    );
    assert_eq!(meta("credentials")["callable_by"]["w1"], "*");
    kernel.shutdown().await;
    fantastic_sqlite_store::close_all(&kernel);
    fantastic_kernel::bootstrap::shutdown(dir.path()).unwrap();
}

#[test]
fn readme_says_how_to_read_usage_per_model_and_agent() {
    let text = readme();
    for word in [
        "\"type\": \"usage\"",
        "`by_model`",
        "`by_agent`",
        "`unpriced: true`",
    ] {
        assert!(text.contains(word), "README's Observe lacks {word}");
    }
}
