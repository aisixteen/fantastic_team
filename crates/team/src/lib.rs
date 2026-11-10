//! fantastic_team as a library: this repo's bundles ([`register_team`]) for
//! fantastic's terminal-shell library (`fantastic_tui::main_with`). The command
//! line is fantastic's.
#![deny(missing_docs)]

pub mod agent_views;
pub mod app_control;
pub mod board;
pub mod cadence_feed;
pub mod cadencer;
pub mod declared_views;
pub mod dispatcher;
pub mod hydrate;
pub mod new_team;
pub mod page;
pub mod puller;
pub mod repo_config;
pub mod team_notice;
pub mod tree;
pub mod webui;
pub mod worker_view;

use fantastic_kernel::BundleRegistry;

pub mod worker_exec;

/// This repo's bundles, added to the registry of fantastic's shell AFTER its
/// stock set (an `fantastic_tui::Extra`): the git and github connectors. The
/// worker exec restriction ([`worker_exec`]) and the config ([`config`]) are records,
/// not bundles; the roles ([`roles`]) are META bundles (yaml over an AI backend, no
/// code) that the roles registry (`agentic_resources.tools`, crates/agentic_resources)
/// spawns from; the task registry (`task_registry.tools`,
/// crates/task_registry) is inert in a team whose registry record has no
/// `task_store_id`.
pub fn register_team(reg: &mut BundleRegistry) {
    // the stock roles, meta bundles over `ai_chat.tools`, which fantastic's stock set registers before this extra (a team's folder may override them)
    if let Err(e) = roles::register(reg) {
        eprintln!("team: a stock role did not register: {e}");
    }
    reg.register(
        team_git_connector::HANDLER_MODULE,
        team_git_connector::GitConnectorBundle,
    );
    reg.register(
        team_github_connector::HANDLER_MODULE,
        team_github_connector::GithubConnectorBundle,
    );
    reg.register(
        team_agentic_resources::HANDLER_MODULE,
        team_agentic_resources::AgenticResourcesBundle,
    );
    reg.register(
        team_task_registry::HANDLER_MODULE,
        team_task_registry::TaskRegistryBundle::default(),
    );
    reg.register(puller::HANDLER_MODULE, puller::PullerBundle::default());
    // The page bundle serves the team page.
    reg.register(page::HANDLER_MODULE, page::PageBundle);
    reg.register(board::HANDLER_MODULE, board::BoardBundle);
    reg.register(dispatcher::HANDLER_MODULE, dispatcher::DispatcherBundle);
    reg.register(
        cadencer::HANDLER_MODULE,
        cadencer::CadencerBundle::default(),
    );
    reg.register(
        worker_view::HANDLER_MODULE,
        worker_view::WorkerViewBundle::default(),
    );
}

/// The README for AIs (`docs/ai-readme.md`), built into the binary and passed to
/// `main_with` as its readme section.
pub const AI_README: &str = include_str!("../../../docs/ai-readme.md");

pub mod roles;

#[cfg(test)]
extern crate self as fantastic_team;

#[cfg(test)]
#[path = "../tests/common/boot.rs"]
mod test_boot;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_team_registers_the_task_registry() {
        let mut reg = BundleRegistry::new();
        register_team(&mut reg);
        assert!(reg.get("task_registry.tools").is_some());
    }

    #[test]
    fn register_team_adds_the_team_bundles_to_any_registry() {
        let mut reg = BundleRegistry::new();
        register_team(&mut reg);
        let names: Vec<&str> = reg.catalog().map(|(n, _)| n).collect();
        assert!(names.contains(&"team_git.tools"));
        assert!(names.contains(&"team_github.tools"));
        // An `fantastic_tui::Extra` is a plain fn over the registry.
        let _extra: fantastic_tui::Extra = register_team;
    }

    #[test]
    fn register_team_registers_the_team_page_bundle() {
        let mut reg = BundleRegistry::new();
        register_team(&mut reg);
        let names: Vec<&str> = reg.catalog().map(|(n, _)| n).collect();
        assert!(names.contains(&"team_page.tools"));
    }

    #[test]
    fn register_team_registers_puller() {
        let mut reg = BundleRegistry::new();
        register_team(&mut reg);
        let names: Vec<&str> = reg.catalog().map(|(n, _)| n).collect();
        assert!(names.contains(&"team_puller.tools"));
    }
}
