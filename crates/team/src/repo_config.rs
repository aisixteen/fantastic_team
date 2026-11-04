//! Per-repo settings: one entry of the `repo_config` field of the puller's record
//! (`owner/name` → settings). A repo opts in by being in the record's `repos`; a repo
//! with no entry takes the defaults. The value is validated here, never executed.
//! Pure: no I/O.

use serde::Deserialize;
use serde_json::Value;
/// The label that marks a task when the file does not name its own.
pub const DEFAULT_LABEL: &str = "team-ready";

/// Who must review a change before it is merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    /// Required approving reviews, at least 1.
    pub approvals: u32,
    /// Logins whose approval counts. Empty means any reviewer.
    pub approvers: Vec<String>,
}

/// The validated configuration of one repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoConfig {
    /// Labels that mark an issue as a task. Never empty.
    pub task_labels: Vec<String>,
    /// Relative globs a worker may touch. Empty means all paths.
    pub paths: Vec<String>,
    /// Build command, if the repo names one.
    pub build: Option<String>,
    /// Test command, if the repo names one.
    pub test: Option<String>,
    /// Review rules.
    pub review: Review,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    #[serde(default)]
    task_labels: Option<Vec<String>>,
    #[serde(default)]
    paths: Option<Vec<String>>,
    #[serde(default)]
    build: Option<String>,
    #[serde(default)]
    test: Option<String>,
    #[serde(default)]
    review: Option<RawReview>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReview {
    #[serde(default)]
    approvals: Option<u32>,
    #[serde(default)]
    approvers: Option<Vec<String>>,
}

/// Validate one repo's entry of the record's `repo_config`. No entry (or `null`, or an
/// empty map) gives the defaults. Errors name the offending key or value.
pub fn from_record(entry: Option<&Value>) -> Result<RepoConfig, String> {
    let raw: Raw = match entry {
        None | Some(Value::Null) => Raw::default(),
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| format!("repo_config: {e}"))?,
    };

    let task_labels = raw
        .task_labels
        .unwrap_or_else(|| vec![DEFAULT_LABEL.to_string()]);
    if task_labels.is_empty() || task_labels.iter().any(|l| l.trim().is_empty()) {
        return Err("repo_config: task_labels must be a non-empty list of label names".into());
    }

    let paths = raw.paths.unwrap_or_default();
    for p in &paths {
        let escapes = p.is_empty()
            || p.starts_with('/')
            || p.contains('\\')
            || p.split('/').any(|seg| seg == "..");
        if escapes {
            return Err(format!(
                "repo_config: path `{p}` must be relative, with no `..` segment"
            ));
        }
    }

    let (approvals, approvers) = match raw.review {
        None => (1, Vec::new()),
        Some(r) => (r.approvals.unwrap_or(1), r.approvers.unwrap_or_default()),
    };
    if approvals == 0 {
        return Err("repo_config: review.approvals must be at least 1".into());
    }

    Ok(RepoConfig {
        task_labels,
        paths,
        build: raw.build,
        test: raw.test,
        review: Review {
            approvals,
            approvers,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(v: Value) -> Result<RepoConfig, String> {
        from_record(Some(&v))
    }

    #[test]
    fn parses_full_config() {
        let cfg = parse(json!({
            "task_labels": ["do-it", "team-ready"],
            "paths": ["src/**", "docs/*.md"],
            "build": "cargo build",
            "test": "cargo test",
            "review": {"approvals": 2, "approvers": ["alice", "bob"]},
        }))
        .unwrap();
        assert_eq!(cfg.task_labels, vec!["do-it", "team-ready"]);
        assert_eq!(cfg.paths, vec!["src/**", "docs/*.md"]);
        assert_eq!(cfg.build.as_deref(), Some("cargo build"));
        assert_eq!(cfg.test.as_deref(), Some("cargo test"));
        assert_eq!(cfg.review.approvals, 2);
        assert_eq!(cfg.review.approvers, vec!["alice", "bob"]);
    }

    #[test]
    fn no_entry_gets_defaults() {
        for cfg in [
            from_record(None).unwrap(),
            parse(Value::Null).unwrap(),
            parse(json!({})).unwrap(),
        ] {
            assert_eq!(cfg.task_labels, vec![DEFAULT_LABEL]);
            assert!(cfg.paths.is_empty());
            assert_eq!(cfg.review.approvals, 1);
            assert!(cfg.build.is_none() && cfg.test.is_none());
        }
    }

    #[test]
    fn rejects_unknown_key() {
        let err = parse(json!({"buidl": "make"})).unwrap_err();
        assert!(err.contains("buidl"), "{err}");
    }

    #[test]
    fn rejects_parent_dir_in_paths() {
        let err = parse(json!({"paths": ["../x"]})).unwrap_err();
        assert!(err.contains("../x"), "{err}");
    }

    #[test]
    fn rejects_zero_approvals() {
        let err = parse(json!({"review": {"approvals": 0}})).unwrap_err();
        assert!(err.contains("approvals"), "{err}");
    }
}
