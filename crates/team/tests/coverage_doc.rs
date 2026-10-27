//! docs/coverage.md maps every closed issue to the tests that prove it (#147).
//! These tests keep the map honest: the table is there, every status is a
//! known word, a `missing` row has an open gap line, and every test the table
//! names still exists, so a renamed or deleted test cannot go unnoticed.

use std::fs;
use std::path::PathBuf;

const HEADER: &str = "issue | behaviour | unit tests | e2e | status";
const STATUSES: [&str; 5] = ["covered", "units only", "e2e only", "missing", "n/a"];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn doc() -> String {
    fs::read_to_string(root().join("docs/coverage.md")).expect("docs/coverage.md must exist")
}

/// The table rows as the cells between the pipes: issue, behaviour, units, e2e, status.
fn rows(text: &str) -> Vec<Vec<String>> {
    text.lines()
        .filter(|l| l.starts_with("| #"))
        .map(|l| {
            l.trim()
                .trim_matches('|')
                .split('|')
                .map(|c| c.trim().to_string())
                .collect()
        })
        .collect()
}

/// The text between backticks in a cell.
fn code_spans(cell: &str) -> Vec<&str> {
    cell.split('`').skip(1).step_by(2).collect()
}

#[test]
fn coverage_doc_exists_and_has_table_header() {
    let text = doc();
    assert!(
        text.lines().any(|l| l.contains(HEADER)),
        "docs/coverage.md lacks the table header `{HEADER}`"
    );
    assert!(!rows(&text).is_empty(), "the table has no issue rows");
}

#[test]
fn every_closed_issue_row_has_a_status_not_missing() {
    let text = doc();
    for row in rows(&text) {
        assert_eq!(row.len(), 5, "row must have five cells: {row:?}");
        let status = row[4].as_str();
        assert!(
            STATUSES.contains(&status),
            "{}: status `{status}` is not one of {STATUSES:?}",
            row[0]
        );
        if status == "missing" {
            let gap = format!("- [ ] {}", row[0]);
            assert!(
                text.lines().any(|l| l.starts_with(&gap)),
                "{} is `missing` but the gaps section has no `{gap}` line",
                row[0]
            );
        }
    }
}

#[test]
fn named_e2e_tests_exist() {
    let mut seen = 0;
    for row in rows(&doc()) {
        for span in code_spans(&row[3]) {
            if let Some(rest) = span.strip_prefix("e2e/tests/") {
                seen += 1;
                assert!(
                    rest.split('/').count() == 2 && root().join(span).is_dir(),
                    "{} cites `{span}`, which is not a folder in the repo",
                    row[0]
                );
            }
        }
    }
    assert!(seen > 0, "the table cites no e2e test");
}

#[test]
fn named_unit_tests_exist() {
    let mut seen = 0;
    for row in rows(&doc()) {
        for span in code_spans(&row[2]) {
            let Some((file, name)) = span.split_once("::") else {
                continue;
            };
            seen += 1;
            let text = fs::read_to_string(root().join(file))
                .unwrap_or_else(|_| panic!("{} cites `{span}`, but {file} is missing", row[0]));
            // Rust tests are functions; shell tests are named in their ok/bad calls.
            let found = if file.ends_with(".rs") {
                text.contains(&format!("fn {name}("))
            } else {
                text.contains(&format!("ok {name}"))
            };
            assert!(found, "{} cites `{span}`, which is not in {file}", row[0]);
        }
    }
    assert!(seen > 0, "the table cites no unit test");
}
