//! Guards the one-fantastic-version rule in Cargo.lock and keeps the notes
//! that describe the pinned fantastic commit in step with it.

use std::collections::BTreeSet;
use std::path::PathBuf;

const SOURCE_PREFIX: &str = "source = \"git+https://github.com/aisixteen/fantastic?branch=main#";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every distinct commit sha named by a fantastic git source in Cargo.lock.
fn locked_fantastic_shas() -> BTreeSet<String> {
    let lock = std::fs::read_to_string(repo_root().join("Cargo.lock")).expect("read Cargo.lock");
    lock.lines()
        .filter_map(|line| line.strip_prefix(SOURCE_PREFIX))
        .filter_map(|rest| rest.strip_suffix('"'))
        .map(str::to_string)
        .collect()
}

#[test]
fn lock_pins_one_fantastic_commit() {
    let shas = locked_fantastic_shas();
    assert_eq!(
        shas.len(),
        1,
        "Cargo.lock must pin exactly one fantastic commit, found {shas:?}; \
         run scripts/update-fantastic.sh to move every fantastic crate together"
    );
}

#[test]
fn notes_name_pinned_commit() {
    let shas = locked_fantastic_shas();
    let sha = shas
        .iter()
        .next()
        .expect("Cargo.lock pins a fantastic commit");
    let short = &sha[..7];
    let notes = std::fs::read_to_string(repo_root().join("docs/fantastic-notes.md"))
        .expect("read docs/fantastic-notes.md");
    assert!(
        notes.contains(short),
        "docs/fantastic-notes.md must name the pinned fantastic commit {short}"
    );
    assert!(
        notes.contains("export_subtree"),
        "docs/fantastic-notes.md must describe the blueprint export_subtree verb (fantastic#357)"
    );
}
