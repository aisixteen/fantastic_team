//! The look the team's own agents' html views share: the parts of the design mock (header,
//! chip, pill, section, key figure, table, bar), drawn with the base's skin tokens only, and a
//! few pure helpers. A view carries no token block: the page that mounts it passes the skin it
//! shows (the base's `fantastic_tui::VIEW_SKINS`, fantastic#698). A view is a template
//! with `{{agent}}` (the agent's id) and `{{helpers}}` (the shared script) in it; [`view`]
//! fills them and returns what an agent's `reflect` carries as its `view`.
#![deny(missing_docs)]

use serde_json::{json, Value};

/// The parts every view draws with, all on the skin's tokens.
pub const COMPONENTS: &str = include_str!("components.css");
/// The shared script: a `// pure` block (tested in node) and the small DOM helpers.
pub const HELPERS: &str = include_str!("helpers.js");

/// The page of a view: the style, then the template with its placeholders filled.
pub fn page(template: &str, agent: &str) -> String {
    format!(
        "<style>\n{COMPONENTS}</style>\n{}",
        template
            .replace("{{helpers}}", HELPERS)
            .replace("{{agent}}", agent)
    )
}

/// The `view` an agent's `reflect` carries: `{kind: "html", bytes, calls}`. `calls` is
/// the verbs the view may call, per agent id.
pub fn view(template: &str, agent: &str, calls: Value) -> Value {
    json!({"kind": "html", "bytes": page(template, agent), "calls": calls})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_fills_the_agent_and_the_helpers_and_carries_no_skin_of_its_own() {
        let p = page("<i>{{agent}}</i><script>{{helpers}}</script>", "cad");
        assert!(p.contains("<i>cad</i>"));
        assert!(!p.contains("{{"));
        assert!(p.contains("function resultClass"));
        // the host passes the skin (fantastic#698): no `--fc-*` value is declared here
        assert!(!p.contains(":root") && !p.contains("--fc-color-page-bg:"));
    }

    /// Every token a part uses is a token of each of the base's view skins.
    #[test]
    fn every_token_a_part_uses_is_a_skin_token() {
        let used: std::collections::BTreeSet<&str> = COMPONENTS
            .split("var(")
            .skip(1)
            .filter_map(|s| s.split([')', ',']).next())
            .filter(|n| n.starts_with("--fc-"))
            .collect();
        assert!(!used.is_empty());
        for name in used {
            for (skin, css) in fantastic_tui::VIEW_SKINS {
                assert!(
                    css.contains(&format!("{name}:")),
                    "{name} is not a token of the {skin} skin"
                );
            }
        }
    }

    #[test]
    fn the_view_names_its_calls() {
        let v = view("x", "a", json!({"a": ["status"]}));
        assert_eq!(v["kind"], "html");
        assert_eq!(v["calls"]["a"], json!(["status"]));
    }
}
