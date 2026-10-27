//! The role cards of the coder team, as files in `roles/` and embedded here.
//! A card is a `---` header (name, purpose, tier, grants) and the directive text;
//! fantastic's roles registry (`agentic_resources`) reads them from the folders
//! `team.yaml` names. A card names a tier KEY (a template of `team.yaml`), never
//! a provider or a model; the text names none either.

/// The worker card (`roles/worker.md`).
pub const WORKER: &str = include_str!("../../../roles/worker.md");
/// The lead card (`roles/lead.md`).
pub const LEAD: &str = include_str!("../../../roles/lead.md");
/// The issue-keeper card (`roles/issue_keeper.md`).
pub const ISSUE_KEEPER: &str = include_str!("../../../roles/issue_keeper.md");
/// The issue-writer card (`roles/issue_writer.md`).
pub const ISSUE_WRITER: &str = include_str!("../../../roles/issue_writer.md");
/// The spec-reviewer card (`roles/spec_reviewer.md`).
pub const SPEC_REVIEWER: &str = include_str!("../../../roles/spec_reviewer.md");
/// The coder project-lead card (`roles/project_lead.md`); it replaces fantastic's
/// generic card of the same name.
pub const PROJECT_LEAD: &str = include_str!("../../../roles/project_lead.md");

/// Every card, by file name.
pub const ALL: [(&str, &str); 6] = [
    ("worker.md", WORKER),
    ("lead.md", LEAD),
    ("issue_keeper.md", ISSUE_KEEPER),
    ("issue_writer.md", ISSUE_WRITER),
    ("spec_reviewer.md", SPEC_REVIEWER),
    ("project_lead.md", PROJECT_LEAD),
];

/// The role names the coder set must hold: a team with `roles:` and a folder
/// that lacks one of them does not start.
pub const CODER_ROLES: [&str; 6] = [
    "worker",
    "lead",
    "issue_keeper",
    "issue_writer",
    "spec_reviewer",
    "project_lead",
];

#[cfg(test)]
fn card_grants_tasks(file: &str) -> bool {
    let text = ALL.iter().find(|(f, _)| *f == file).unwrap().1;
    fantastic_agentic_resources::cards::parse_card(text, file)
        .unwrap()
        .grants
        .contains_key("tasks")
}

#[cfg(test)]
mod tests {
    use super::*;
    use fantastic_agentic_resources::cards::{self, parse_card};
    use std::collections::BTreeSet;

    fn card(file: &str) -> cards::Card {
        let text = ALL.iter().find(|(f, _)| *f == file).unwrap().1;
        parse_card(text, file).unwrap_or_else(|e| panic!("{e}"))
    }

    fn words(s: &str) -> Vec<String> {
        s.to_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(String::from)
            .collect()
    }

    const DENY: [&str; 9] = [
        "claude",
        "haiku",
        "sonnet",
        "opus",
        "qwen",
        "gpt",
        "llama",
        "anthropic",
        "openai",
    ];

    #[test]
    fn cards_parse_and_have_the_five_coder_names_plus_project_lead() {
        let names: BTreeSet<String> = ALL
            .iter()
            .map(|(f, t)| parse_card(t, f).unwrap().name)
            .collect();
        let want: BTreeSet<String> = CODER_ROLES.iter().map(|s| s.to_string()).collect();
        assert_eq!(names, want);
        assert_eq!(ALL.len(), 6);
    }

    #[test]
    fn moved_cards_keep_the_old_text() {
        let (w, l, k) = (card("worker.md"), card("lead.md"), card("issue_keeper.md"));
        assert!(w.directive.starts_with("# The worker directive"));
        assert!(l.directive.starts_with("# The lead directive"));
        assert!(k.directive.starts_with("# The issue-keeper directive"));
        for s in [
            "if_match",
            "version",
            "version_mismatch",
            ".state",
            "pending",
            "taken",
            "until",
            "done",
            "failed",
            "\"taker\":\"YOU\"",
            "<type>list</type>",
            "<type>read</type>",
            "<type>write</type>",
        ] {
            assert!(w.directive.contains(s), "worker card lacks {s}");
        }
        for s in [
            "if_match",
            "version_mismatch",
            "<type>list</type>",
            "<type>read</type>",
            "<type>write</type>",
            "delete_agent",
            "wait_s",
            "generating",
            "fantastic#227",
            "context_clear",
            "Before handing a worker a NEW task",
            "A nudge on the same task keeps its context",
            "in two turns",
            "Calls in one turn run at the same time",
            "Turn 1",
            "Turn 2",
        ] {
            assert!(l.directive.contains(s), "lead card lacks {s}");
        }
        for t in [&w.directive, &l.directive] {
            assert!(t.contains("wait_s"));
            assert!(t.contains("await"));
            assert!(t.contains("<call name=\"done\"></call>"));
            for old in [
                "<type>take</type>",
                "<type>renew</type>",
                "<type>release</type>",
                "<type>done</type>",
            ] {
                assert!(!t.contains(old), "a card still uses {old}");
            }
        }
        let text = k.directive.to_lowercase();
        for s in ["never merge", "never close", "never edit", "never create"] {
            assert!(text.contains(s), "issue-keeper card lacks '{s}'");
        }
        assert!(text.contains("spec"));
        let ww = words(&w.directive);
        assert!(!ww.iter().any(|x| x == "git" || x == "github"));
    }

    #[test]
    fn cards_name_no_provider_or_model() {
        for (file, text) in ALL {
            let c = parse_card(text, file).unwrap();
            for d in DENY {
                assert!(!text.to_lowercase().contains(d), "{file} names {d}");
            }
            assert!(
                ["small", "strong", "issue_keeper"].contains(&c.tier.as_str()),
                "{file}: tier {}",
                c.tier
            );
        }
        assert_eq!(card("worker.md").tier, "small");
        assert_eq!(card("lead.md").tier, "strong");
        assert_eq!(card("issue_keeper.md").tier, "issue_keeper");
        assert_eq!(card("project_lead.md").tier, "strong");
    }

    #[test]
    fn lead_card_spawns_and_asks_roles_without_an_admit_step() {
        let l = card("lead.md").directive;
        let start = l.find("## Starting an agent").unwrap();
        let end = l.find("## Workers that serve themselves").unwrap();
        let section = &l[start..end];
        assert!(section.contains("Ask `roles` first (roles, then get_count)"));
        assert!(section.contains("<type>spawn</type>"));
        assert!(section.contains("get_count"));
        assert!(!section.contains("create_agent"));
        assert!(!section.contains("Admit it"));
        assert!(!section.contains("callable_by"));
        assert!(!section.contains("update_agent"));
        assert!(section.contains("fleet"));
        assert!(section.contains("The queue is QUEUE. Your id is the id in your own reflect."));
        assert!(section.contains("error:\"limit\""));
        assert!(l.contains("Ask `roles` before starting an agent"));
        let g = card("lead.md").grants;
        assert_eq!(
            g["$registry"],
            serde_json::json!(["roles", "role", "spawn", "get_count", "fleet", "report"])
        );
        assert!(g.contains_key("kernel") && g.contains_key("queue"));
    }

    #[test]
    fn project_lead_card_grants_the_connector_and_board() {
        let c = card("project_lead.md");
        assert_eq!(
            c.grants["github"],
            serde_json::json!([
                "issues",
                "issue_view",
                "issue_edit",
                "issue_state",
                "comment",
                "label",
                "project_field_set",
                "pr_create",
                "pr_status",
                "staleness",
                "push"
            ])
        );
        assert_eq!(c.grants["board"], serde_json::json!(["board", "rows"]));
        assert_eq!(
            c.grants["kernel_state"],
            serde_json::json!(["await_set", "await_cancel", "create_agent", "delete_agent"]),
            "the card tells it to wait with await_set and may make a sidecar"
        );
        assert!(c.grants["$registry"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("spawn")));
        assert!(c.grants.contains_key("queue"));
        for s in [
            "await_set",
            "fork_points",
            "pack understood",
            "only after \"yes\"",
            "Nothing is cleaned up until the user asks",
            "one pull request at a time",
            "pr_create",
            "get_count",
        ] {
            assert!(c.directive.contains(s), "project_lead card lacks {s}");
        }
    }

    #[test]
    fn project_lead_card_records_versions_and_checks_staleness() {
        let c = card("project_lead.md");
        assert!(c.grants["github"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("staleness")));
        for s in [
            "staleness",
            "versions",
            "source_version",
            "fork_from",
            "Code gives the facts",
            "5 minutes",
            "1-hour",
            "burst",
            "fork_points",
            "cache_read_tokens",
            "usage",
        ] {
            assert!(c.directive.contains(s), "project_lead card lacks {s}");
        }
        // no new registry verbs: the grants stay as they were
        assert_eq!(
            c.grants["tasks"],
            serde_json::json!(["add", "assign", "reassign", "status", "report", "list", "get"])
        );
    }

    #[test]
    fn project_lead_card_does_not_talk_of_an_admit() {
        // spawn opens the card's tools on both ends (fantastic#436): no card tells an
        // agent to wait for, or to perform, an admit
        for file in ["project_lead.md", "lead.md"] {
            let d = card(file).directive;
            for s in ["admit", "not admitted", "Admit it"] {
                assert!(!d.contains(s), "{file} still says `{s}`");
            }
        }
    }

    #[test]
    fn project_lead_card_grants_the_task_registry() {
        let c = card("project_lead.md");
        assert_eq!(
            c.grants["tasks"],
            serde_json::json!(["add", "assign", "reassign", "status", "report", "list", "get"])
        );
        for s in [
            "gh:",
            "update_to",
            "reassign",
            "holder",
            "subtree",
            "nothing syncs automatically",
            "YOURSELF",
        ] {
            assert!(c.directive.contains(s), "project_lead card lacks {s}");
        }
        for card in ["lead.md", "worker.md"] {
            assert!(!super::card_grants_tasks(card), "{card} grants tasks");
        }
    }

    #[test]
    fn project_lead_card_decides_when_to_ask() {
        let d = card("project_lead.md").directive;
        for s in [
            "decide when to ask",
            "large plan",
            "past your limits",
            "any GitHub write",
            "issue_edit",
            "pr_create",
            "the user's reply in the chat",
            "show me the plan first",
            "wait for \"yes\" before starting anything",
        ] {
            assert!(d.contains(s), "project_lead card lacks {s}");
        }
        assert!(
            !d.contains("START NOTHING yet"),
            "the unconditional gate is gone"
        );
    }

    #[test]
    fn cards_use_the_removed_task_lines_no_more() {
        let pl = card("project_lead.md").directive;
        assert!(!pl.contains("waits_on"));
        assert!(!pl.contains("{label"));
        assert!(!card("lead.md").directive.contains("state in `fleet`"));
    }

    #[test]
    fn the_repo_roles_folder_loads_clean() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../roles").to_string();
        let (loaded, errors) = cards::load_dir(&[dir]);
        assert!(errors.is_empty(), "{errors:?}");
        let names: BTreeSet<String> = loaded.iter().map(|c| c.name.clone()).collect();
        let want: BTreeSet<String> = CODER_ROLES.iter().map(|s| s.to_string()).collect();
        assert_eq!(names, want);
        let pl = loaded.iter().find(|c| c.name == "project_lead").unwrap();
        assert!(
            pl.grants.contains_key("github"),
            "the coder card replaces the generic one"
        );
    }

    #[test]
    fn worker_lead_and_project_lead_cards_grant_sidecar_creation() {
        for file in ["worker.md", "lead.md", "project_lead.md"] {
            let g = card(file).grants;
            let k = g["kernel_state"].as_array().unwrap();
            for v in ["create_agent", "delete_agent"] {
                assert!(k.contains(&serde_json::json!(v)), "{file} lacks {v}");
            }
            assert_ne!(g["kernel_state"], "*", "{file}");
            assert_eq!(
                g["canvas_page"],
                serde_json::json!(["new_canvas"]),
                "{file}"
            );
        }
        assert_eq!(
            card("lead.md").grants["kernel"],
            "*",
            "the lead card's kernel grant is unchanged"
        );
    }

    #[test]
    fn service_style_cards_grant_no_sidecars() {
        for file in ["issue_keeper.md", "issue_writer.md", "spec_reviewer.md"] {
            let c = card(file);
            assert!(!c.grants.contains_key("canvas_page"), "{file}");
            assert!(!c.grants.contains_key("kernel"), "{file}");
            assert!(!c.directive.contains("new_canvas"), "{file}");
            assert!(!c.directive.contains("## Sidecars and canvases"), "{file}");
        }
    }

    #[test]
    fn cards_tell_when_to_embed_and_when_to_make_a_canvas() {
        for file in ["worker.md", "lead.md", "project_lead.md"] {
            let d = card(file).directive;
            let at = d
                .find("## Sidecars and canvases")
                .unwrap_or_else(|| panic!("{file}"));
            let section = &d[at..];
            for s in [
                "embed",
                "unembed",
                "new_canvas",
                "nested",
                "delete",
                "YOUR OWN ID",
            ] {
                assert!(section.contains(s), "{file} lacks {s}");
            }
            assert!(
                section.contains("Never put workers, leads or services on a canvas"),
                "{file}"
            );
        }
    }

    #[test]
    fn lead_card_starting_an_agent_section_still_has_no_create_agent() {
        let l = card("lead.md").directive;
        let start = l.find("## Starting an agent").unwrap();
        let end = l.find("## Workers that serve themselves").unwrap();
        assert!(!l[start..end].contains("create_agent"));
        // the sidecar text is its own section, outside it, and says it is about sidecars only
        let side = l.find("## Sidecars and canvases").unwrap();
        assert!(side > end);
        assert!(l[side..].contains("sidecars only"));
        assert!(l[side..].contains("never create a worker or a lead with `create_agent`"));
    }
}
