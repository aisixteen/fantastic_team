//! The app's preset: a BLUEPRINT FOLDER in roles mode (a lead, the roles registry with
//! the tiers and limits, the `tasks` registry, the queue, the board and the GitHub
//! connector) and the coder role cards. The app writes both when it creates a team and
//! then imports the blueprint under the home's `user` agent, the way any blueprint is
//! imported (the `user` agent's `import_subtree`, `fantastic_team import`). The CLI never
//! calls this: there the caller brings the records, a blueprint folder or a directory
//! that already holds `.fantastic/agents/`, and nothing is created behind it.
//!
//! fantastic's import gives every agent a fresh id (`lead` becomes `lead_1`), and the
//! role cards name the agents they grant by id, so [`bind_cards`] writes the new ids
//! into the cards from the import's reply.

use std::path::Path;

use serde_json::{json, Value};

use crate::roles;

/// The preset blueprint (`preset/app_team/blueprint.yaml`).
pub const APP_TEAM_BLUEPRINT: &str = include_str!("../preset/app_team/blueprint.yaml");

/// The folder, under the team's directory, the blueprint is written to.
pub const BLUEPRINT_DIR: &str = "blueprint";

/// The agent the preset is imported under: the home's `user`.
pub const PARENT: &str = "user";

/// The ports of the blueprint and the home agents they are bound to.
pub const BINDS: [(&str, &str); 3] = [
    ("@memory", "memory"),
    ("@files", "files"),
    ("@credentials", "credentials"),
];

/// The agents of the blueprint a role card names (by id) in its grants or its text.
pub const CARD_AGENTS: [&str; 4] = ["github", "board", "queue", "tasks"];

/// Write the preset into `dir`: the blueprint folder `blueprint/` and the coder role
/// cards under `roles/`. Returns `Ok(false)` and writes nothing when `blueprint/` is
/// already there, so the user's edits are never overwritten; `Ok(true)` when it wrote
/// the files.
pub fn write_app_preset(dir: &Path) -> Result<bool, String> {
    let folder = dir.join(BLUEPRINT_DIR);
    if folder.exists() {
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
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("cannot create {}: {e}", folder.display()))?;
    let file = folder.join("blueprint.yaml");
    std::fs::write(&file, APP_TEAM_BLUEPRINT)
        .map_err(|e| format!("cannot write {}: {e}", file.display()))?;
    Ok(true)
}

/// The `send` item that imports the written blueprint, as `fantastic_team import
/// blueprint --parent user --bind … --approve` does: the `user` agent's `import_subtree`.
pub fn import_item() -> Value {
    let bind: serde_json::Map<String, Value> = BINDS
        .iter()
        .map(|(p, a)| (p.to_string(), json!(a)))
        .collect();
    json!({"target": "user", "payload": {
        "type": "import_subtree", "path": BLUEPRINT_DIR, "parent_id": PARENT,
        "bind": bind, "approve": true,
    }})
}

/// The two tool agents the cards grant besides the blueprint's own, as `send` items for the
/// root (`create_agent`) under `lead`: `worker_exec` (the restricted exec of
/// [`crate::worker_exec`]) and `worker_files` (a file bridge on the work folder). They
/// carry the absolute paths of `dir`, which a blueprint may not hold, so they are made
/// with `send`, after the import, with the ids the cards name.
pub fn tool_items(dir: &Path, lead: &str) -> Vec<Value> {
    let work = crate::worker_exec::work_dir(dir)
        .to_string_lossy()
        .into_owned();
    let mut exec = crate::worker_exec::worker_exec_record(dir);
    let head = |rec: &mut Value, id: &str| {
        rec["type"] = json!("create_agent");
        rec["id"] = json!(id);
        rec["parent_id"] = json!(lead);
        rec["callable_by"] = json!({"$self": "*", "$parent": "*"});
    };
    head(&mut exec, "worker_exec");
    let mut files = json!({"handler_module": "file_bridge.tools", "root": work, "allow": ["."],
        "ingress_rule": "allow_all_ingress"});
    head(&mut files, "worker_files");
    vec![
        json!({"target": crate::tree::ROOT_ID, "payload": exec}),
        json!({"target": crate::tree::ROOT_ID, "payload": files}),
    ]
}

/// Write the new ids into the role cards under `dir/roles`: `ids` is the import reply's
/// `ids` (blueprint id → fresh id). In each card the grants keys and the backticked ids
/// of [`CARD_AGENTS`] become the fresh ids. Returns how many cards changed.
pub fn bind_cards(dir: &Path, ids: &Value) -> Result<usize, String> {
    let map = ids.as_object().ok_or("the import reply has no `ids` map")?;
    let mut changed = 0;
    let cards = dir.join("roles");
    for (file, _) in roles::ALL {
        let path = cards.join(file);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut out = text.clone();
        for agent in CARD_AGENTS {
            let Some(fresh) = map.get(agent).and_then(Value::as_str) else {
                continue;
            };
            out = out
                .replace(&format!("\n  {agent}:"), &format!("\n  {fresh}:"))
                .replace(&format!("`{agent}`"), &format!("`{fresh}`"));
        }
        if out != text {
            std::fs::write(&path, out)
                .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            changed += 1;
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fantastic_kernel::bootstrap;
    use fantastic_kernel::Item;
    use std::path::PathBuf;

    #[test]
    fn the_preset_is_a_blueprint_folder_not_a_team_file() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(write_app_preset(dir.path()).unwrap());
        assert!(dir
            .path()
            .join(BLUEPRINT_DIR)
            .join("blueprint.yaml")
            .is_file());
        for (file, _) in roles::ALL {
            assert!(dir.path().join("roles").join(file).is_file(), "{file}");
        }
        let top = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(top, 2, "only `blueprint/` and `roles/` are written");
    }

    #[test]
    fn write_app_preset_never_overwrites() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(write_app_preset(dir.path()).unwrap());
        let path = dir.path().join(BLUEPRINT_DIR).join("blueprint.yaml");
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

    #[test]
    fn the_blueprint_modules_all_resolve_in_the_registry() {
        let doc: Value = serde_yaml::from_str(APP_TEAM_BLUEPRINT).unwrap();
        fn modules(v: &Value, out: &mut Vec<String>) {
            if let Some(m) = v.get("handler_module").and_then(Value::as_str) {
                out.push(m.to_string());
            }
            for k in v
                .get("agents")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                modules(k, out);
            }
        }
        let mut found = Vec::new();
        for top in doc["agents"].as_array().unwrap() {
            modules(top, &mut found);
        }
        for tier in doc["agents"][0]["agents"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|a| a.get("tiers").and_then(Value::as_object))
            .flat_map(|t| t.values())
        {
            found.push(tier["handler_module"].as_str().unwrap().to_string());
        }
        let reg = crate::registry();
        let missing: Vec<&String> = found.iter().filter(|m| reg.get(m).is_none()).collect();
        assert!(missing.is_empty(), "not registered: {missing:?}");
    }

    #[test]
    fn bind_cards_writes_the_fresh_ids_into_the_grants_and_the_text() {
        let dir = tempfile::TempDir::new().unwrap();
        write_app_preset(dir.path()).unwrap();
        let ids = json!({"github": "github_1", "board": "board_1", "queue": "queue_1",
            "tasks": "tasks_1", "lead": "lead_1"});
        assert!(bind_cards(dir.path(), &ids).unwrap() >= 1);
        let card = std::fs::read_to_string(dir.path().join("roles/project_lead.md")).unwrap();
        for fresh in ["github_1", "board_1", "queue_1", "tasks_1"] {
            assert!(card.contains(&format!("\n  {fresh}:")), "{fresh} grant");
            assert!(card.contains(&format!("`{fresh}`")), "{fresh} text");
        }
        assert!(!card.contains("\n  github:"), "the old grant key is gone");
        assert!(
            card.contains("kernel_state:"),
            "ids outside the blueprint stay"
        );
        assert!(bind_cards(dir.path(), &json!("no")).is_err());
    }

    /// The real thing: seed a home the way the CLI does, boot it with this repo's bundles, and
    /// import the preset through the `user` agent's `import_subtree`, as the app does.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_preset_blueprint_imports_into_a_seeded_home() {
        let dir = tempfile::TempDir::new().unwrap();
        fantastic_tui::seed_dir(dir.path()).unwrap();
        write_app_preset(dir.path()).unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        // the home's `credentials` agent (`fantastic ai …` makes it when a key is saved)
        let mk = json!({"type": "create_agent", "parent_id": "user", "id": "credentials",
            "handler_module": "credential_store.tools", "backend": "env", "refs": {}});
        let o = kernel
            .send(vec![Item::new(
                fantastic_kernel::bootstrap::DEFAULT_ROOT_ID,
                mk,
            )])
            .await;
        assert!(o[0].result.get("error").is_none(), "{}", o[0].result);
        let mut item = import_item();
        item["payload"]["path"] = json!(dir.path().join(BLUEPRINT_DIR).to_string_lossy());
        let out = kernel
            .send(vec![Item::new("user", item["payload"].clone())])
            .await;
        let reply = out[0].result.clone();
        assert!(reply.get("error").is_none(), "{reply}");
        let ids = &reply["ids"];
        for id in ["lead", "queue", "github", "board", "roles", "tasks"] {
            let fresh = ids[id]
                .as_str()
                .unwrap_or_else(|| panic!("no id for {id}: {reply}"));
            assert!(
                kernel
                    .agents
                    .contains_key(&fantastic_kernel::AgentId::from(fresh)),
                "{fresh} was not created"
            );
        }
        let meta = |id: &str| {
            kernel
                .agents
                .get(&fantastic_kernel::AgentId::from(ids[id].as_str().unwrap()))
                .map(|a| a.meta.read().unwrap().clone())
                .unwrap()
        };
        // the registry is the TIERS and LIMITS record, not a file
        let roles_rec = meta("roles");
        assert_eq!(roles_rec["tiers"]["strong"]["model"], "claude-opus-5-5");
        assert_eq!(roles_rec["limits"]["total"], 3);
        // the lead: its backend fields, its memory in the home's store, its directive a file
        let lead = meta("lead");
        assert_eq!(lead["memory_store_id"], "memory", "{lead:?} {reply}");
        assert_eq!(lead["system_prompt_file"], "roles/lead.md");
        assert_eq!(lead["credentials"]["store"], "credentials");
        assert!(lead["can_call"]
            .get(ids["roles"].as_str().unwrap())
            .is_some());
        assert_eq!(meta("tasks")["task_store_id"], "memory");
        // the cards follow the fresh ids
        bind_cards(dir.path(), ids).unwrap();
        let card = std::fs::read_to_string(dir.path().join("roles/project_lead.md")).unwrap();
        assert!(card.contains(&format!("\n  {}:", ids["github"].as_str().unwrap())));
        // the tools the cards grant, made with `send` (they carry absolute paths)
        let lead_id = ids["lead"].as_str().unwrap();
        for item in tool_items(dir.path(), lead_id) {
            let o = kernel
                .send(vec![Item::new(
                    item["target"].as_str().unwrap(),
                    item["payload"].clone(),
                )])
                .await;
            assert!(o[0].result.get("error").is_none(), "{}", o[0].result);
        }
        // a worker is spawned through the registry: the tier's backend fields and the
        // card's grants on the fresh ids
        let roles_id = ids["roles"].as_str().unwrap();
        // `role_dirs: [roles]` is relative to the directory the team runs in; the test
        // process runs elsewhere, so point the registry at this directory's cards
        let o = kernel
            .send(vec![Item::new(
                fantastic_kernel::bootstrap::DEFAULT_ROOT_ID,
                json!({"type": "update_agent", "id": roles_id,
                       "role_dirs": [dir.path().join("roles").to_string_lossy()]}),
            )])
            .await;
        assert!(o[0].result.get("error").is_none(), "{}", o[0].result);
        let out = kernel
            .send(vec![Item::new(
                roles_id,
                json!({"type": "spawn", "role": "worker"}),
            )])
            .await
            .remove(0)
            .result;
        let worker = out["id"]
            .as_str()
            .unwrap_or_else(|| panic!("spawn: {out}"))
            .to_string();
        assert_eq!(out["tier"], "small", "{out}");
        let w = kernel
            .agents
            .get(&fantastic_kernel::AgentId::from(worker.as_str()))
            .map(|a| a.meta.read().unwrap().clone())
            .unwrap();
        assert_eq!(w["model"], "claude-haiku-5-5");
        assert_eq!(w["memory_store_id"], "memory");
        let queue_id = ids["queue"].as_str().unwrap();
        for tool in ["worker_exec", "worker_files", queue_id] {
            assert_eq!(w["can_call"][tool], "*", "{tool}: {w:?}");
        }
        // the limit of the record is read: three in all, two workers
        let count = kernel
            .send(vec![Item::new(roles_id, json!({"type": "get_count"}))])
            .await
            .remove(0)
            .result;
        assert!(count.to_string().contains('3'), "{count}");
        kernel.shutdown().await;
        fantastic_sqlite_store::close_all(&kernel);
        bootstrap::shutdown(dir.path()).unwrap();
    }
}
