//! The pack browser panel: an html view that lists the packs, shows a pack's
//! permission preview, previews and runs an install, and runs the evals. It
//! sends only the verbs `pack_listing` (`list`, `show`) and `pack` (`install`,
//! `eval`) already answer, and draws every value as text. The html is kept in the
//! binary; it is not an agent (the team page serves the dispatcher and its Pack
//! install panel).

/// The panel's page, embedded in the binary.
pub const HTML: &str = include_str!("../web/packs.html");
