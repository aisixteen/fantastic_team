//! The look the team's own agents' html views share: the day skin's tokens (one `:root`
//! block, the same one the base views carry), the parts of the design mock (header, chip,
//! pill, section, key figure, table, bar) and a few pure helpers. A view is a template
//! with `{{agent}}` (the agent's id) and `{{helpers}}` (the shared script) in it; [`view`]
//! fills them and returns what an agent's `reflect` carries as its `view`.
#![deny(missing_docs)]

use serde_json::{json, Value};

/// The day skin's `:root{--fc-...}` block.
pub const TOKENS: &str = include_str!("tokens.css");
/// The parts every view draws with, all on the tokens.
pub const COMPONENTS: &str = include_str!("components.css");
/// The shared script: a `// pure` block (tested in node) and the small DOM helpers.
pub const HELPERS: &str = include_str!("helpers.js");

/// The page of a view: the style, then the template with its placeholders filled.
pub fn page(template: &str, agent: &str) -> String {
    format!(
        "<style>\n{TOKENS}\n{COMPONENTS}</style>\n{}",
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
    fn the_page_fills_the_agent_and_the_helpers_and_leads_with_one_root_block() {
        let p = page("<i>{{agent}}</i><script>{{helpers}}</script>", "cad");
        assert!(p.contains("<i>cad</i>"));
        assert!(!p.contains("{{"));
        assert!(p.contains("function resultClass"));
        assert_eq!(p.matches(":root{").count(), 1);
    }

    #[test]
    fn every_token_a_part_uses_is_declared() {
        let used: std::collections::BTreeSet<&str> = COMPONENTS
            .split("var(")
            .skip(1)
            .filter_map(|s| s.split([')', ',']).next())
            .filter(|n| n.starts_with("--fc-"))
            .collect();
        assert!(!used.is_empty());
        for name in used {
            assert!(
                TOKENS.contains(&format!("{name}:")),
                "{name} is not declared"
            );
        }
    }

    #[test]
    fn the_view_names_its_calls() {
        let v = view("x", "a", json!({"a": ["status"]}));
        assert_eq!(v["kind"], "html");
        assert_eq!(v["calls"]["a"], json!(["status"]));
    }
}
