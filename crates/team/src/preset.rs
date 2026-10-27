//! The app's preset: a `team.yaml` in roles mode (a lead, the coder role cards, the
//! `tasks` registry and limits). The app writes it when it creates a team; the user
//! edits it. The CLI never calls this: there the caller writes `team.yaml` and
//! nothing is created behind it.

use std::path::Path;

use crate::{config, roles};

/// The preset `team.yaml` (`preset/app_team.yaml`).
pub const APP_TEAM_YAML: &str = include_str!("../preset/app_team.yaml");

/// Write the preset into `dir`: `team.yaml` and the coder role cards under `roles/`.
/// Returns `Ok(false)` and writes nothing when `team.yaml` is already there, so the
/// user's edits are never overwritten; `Ok(true)` when it wrote the files.
pub fn write_app_preset(dir: &Path) -> Result<bool, String> {
    let path = dir.join(config::FILE_NAME);
    if path.exists() {
        return Ok(false);
    }
    let cards = dir.join("roles");
    std::fs::create_dir_all(&cards)
        .map_err(|e| format!("cannot create {}: {e}", cards.display()))?;
    for (file, text) in roles::ALL {
        let p = cards.join(file);
        // a card the user already has stays as it is
        if !p.exists() {
            std::fs::write(&p, text).map_err(|e| format!("cannot write {}: {e}", p.display()))?;
        }
    }
    std::fs::write(&path, APP_TEAM_YAML)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree;
    use fantastic_kernel::bootstrap;
    use std::path::PathBuf;

    #[test]
    fn app_preset_parses_in_roles_mode() {
        let cfg = config::parse(APP_TEAM_YAML).unwrap();
        let roles = cfg.roles.as_ref().expect("roles mode");
        assert!(cfg.first_agent.is_some());
        for t in ["strong", "small", "issue_keeper"] {
            assert!(cfg.templates.contains_key(t), "{t}");
        }
        assert!(roles.limits.total.is_some());
        assert!(cfg.board.is_some());
    }

    #[test]
    fn app_preset_plans_with_the_shipped_cards() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(write_app_preset(dir.path()).unwrap());
        let cfg = config::load(&dir.path().join(config::FILE_NAME)).unwrap();
        let plan = tree::plan(&cfg, dir.path()).unwrap();
        for id in ["roles", "memory", "tasks", "lead", "dispatcher"] {
            assert!(plan.iter().any(|p| p.id == id), "the plan has no `{id}`");
        }
        for (file, _) in roles::ALL {
            assert!(dir.path().join("roles").join(file).is_file(), "{file}");
        }
    }

    #[test]
    fn write_app_preset_never_overwrites() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(write_app_preset(dir.path()).unwrap());
        let path = dir.path().join(config::FILE_NAME);
        std::fs::write(&path, "# mine\n").unwrap();
        assert!(!write_app_preset(dir.path()).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# mine\n");
    }

    #[test]
    fn preset_cards_equal_the_repo_cards() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roles");
        for (file, text) in roles::ALL {
            let on_disk = std::fs::read_to_string(repo.join(file)).unwrap();
            assert_eq!(text, on_disk, "{file} drifted from roles/{file}");
        }
        let dir = tempfile::TempDir::new().unwrap();
        write_app_preset(dir.path()).unwrap();
        for (file, text) in roles::ALL {
            let written = std::fs::read_to_string(dir.path().join("roles").join(file)).unwrap();
            assert_eq!(written, text, "{file}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn app_preset_starts_in_roles_mode() {
        let dir = tempfile::TempDir::new().unwrap();
        write_app_preset(dir.path()).unwrap();
        let cfg = config::load(&dir.path().join(config::FILE_NAME)).unwrap();
        let plan = tree::plan(&cfg, dir.path()).unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        let applied = tree::apply(&kernel, &plan).await.unwrap();
        tree::apply_limits(&kernel, &cfg).await.unwrap();
        for id in ["roles", "tasks"] {
            assert!(
                applied.created.iter().any(|c| c == id),
                "`{id}` not created"
            );
            assert!(kernel
                .agents
                .contains_key(&fantastic_kernel::AgentId::from(id)));
        }
        kernel.shutdown().await;
        bootstrap::shutdown(dir.path()).unwrap();
    }
}
