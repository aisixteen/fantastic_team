//! The load and progress view (#127): the code gives plain counts, the owning
//! agent decides what to show. This test checks that no mode, enum or
//! progress/load switch exists in the load code.
//! The digest a scripted driver receives is tested in `cadencer.rs`, through
//! the real beat and delivery path.

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
