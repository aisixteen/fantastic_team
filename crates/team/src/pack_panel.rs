//! The pack browser panel: an html view that lists the packs, shows a pack's
//! permission preview, previews and runs an install, and runs the evals. It
//! sends only the verbs `pack_listing` (`list`, `show`) and `pack` (`install`,
//! `eval`) already answer, and draws every value as text.

/// The panel's page, embedded in the binary.
pub const HTML: &str = include_str!("../web/packs.html");

/// The panel's agent id: an html view on the canvas, made at the first start.
pub const VIEW_ID: &str = "packs_view";
