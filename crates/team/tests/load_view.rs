//! The load and progress view (#127): the code gives plain counts, the owning
//! agent decides what to show. These tests check that no mode, enum or
//! progress/load switch exists in the load code, and that no total is made.
//! The digest a scripted driver receives is tested in `cadencer.rs`, through
//! the real beat and delivery path.

use std::collections::BTreeMap;

use fantastic_team::board::{load_numbers, Board, Group, Live, Status, Task};

/// The files the load code lives in.
const LOAD_SOURCES: &[(&str, &str)] = &[
    ("board.rs", include_str!("../src/board.rs")),
    ("cadencer.rs", include_str!("../src/cadencer.rs")),
    ("cadence_feed.rs", include_str!("../src/cadence_feed.rs")),
    ("dispatcher.rs", include_str!("../src/dispatcher.rs")),
];

#[test]
fn no_mode_field_or_enum_for_load() {
    for forbidden in ["enum LoadMode", "enum View", "enum Progress", "enum Load"] {
        for (file, src) in LOAD_SOURCES {
            assert!(!src.contains(forbidden), "{forbidden} found in {file}");
        }
    }
    for (file, src) in LOAD_SOURCES {
        for line in src.lines().filter(|l| l.contains("load")) {
            for word in ["\"mode\"", "\"pipe\"", "\"list\""] {
                assert!(
                    !line.contains(word),
                    "{word} next to load in {file}: {line}"
                );
            }
        }
    }
}

#[test]
fn load_numbers_has_no_total_or_of_key() {
    let mut b = Board::default();
    b.tasks.insert(
        "o/r#1".into(),
        Task {
            id: "o/r#1".into(),
            status: Status::Pending,
            ..Default::default()
        },
    );
    let groups: BTreeMap<String, Group> = BTreeMap::new();
    let mut live = BTreeMap::new();
    live.insert("w1".to_string(), Live::default());
    b.touch_agent("w1", "idle", 0.0, 100);
    let load = load_numbers(&b, &groups, &live, 100, 300);
    let text = load.to_string();
    assert!(!text.contains("\"total\""), "{text}");
    assert!(!text.contains("\"of\""), "{text}");
    assert!(load["tasks"].get("waiting_ci").is_none());
}
