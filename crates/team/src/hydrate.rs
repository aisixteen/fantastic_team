//! The team hydrates the way fantastic does (#261): its fixed agents are written ONCE, at
//! the first run, from a blueprint, and every later run rehydrates them from disk.
//!
//! * FIRST RUN: [`seed`] is fantastic's `seed_dir_with` over [`BLUEPRINT`]
//!   (`crates/team/blueprint.yaml`), before the kernel boots; ids as written, the grants and
//!   the config as the yaml says (fantastic `src/tui/src/boot/seed.rs`, `seed_dir_with`
//!   and `write_blueprint`).
//! * EVERY RUN AFTER: the fs loader (`kernel_state`) is the root loader. It rehydrates
//!   `.fantastic/agents/**/agent.yaml` and writes every change back, and fantastic's own
//!   boot (`kern/host.rs`, `Host::boot`) does the rest: `reach::grant_all` writes the
//!   human's and every AI agent's edges on both ends, and `bundle_watch` follows the
//!   bundles. The blueprint is never read again, so an agent deleted from disk stays
//!   deleted and a field changed on disk stays changed, exactly as for fantastic's own
//!   stock agents (`seed.rs`, module doc: "Every later run rehydrates what is on disk and
//!   never reads the blueprint again").
//!
//! The team has no boot code of its own for this: nothing creates, updates or grants an
//! agent at a boot.

use std::path::Path;

use serde_json::Value;

/// The team's fixed agents, compiled in.
pub const BLUEPRINT: &str = include_str!("../blueprint.yaml");

/// The team folder's work dir, as a blueprint value: the one value that is not the same on
/// every machine (an absolute path), so the first run fills it in.
const WORK_PLACEHOLDER: &str = "@work";

/// The blueprint with the first run's placeholders of `dir` filled in. `@root` stays: the
/// seed writer resolves it.
fn filled(text: &str, dir: &Path) -> Result<Value, String> {
    let mut doc: Value = serde_yaml::from_str(text).map_err(|e| format!("blueprint: {e}"))?;
    fill(&mut doc, dir);
    Ok(doc)
}

fn fill(v: &mut Value, dir: &Path) {
    match v {
        Value::String(s) if s == WORK_PLACEHOLDER => {
            *s = crate::worker_exec::work_dir(dir)
                .to_string_lossy()
                .into_owned();
        }
        Value::Array(a) => a.iter_mut().for_each(|x| fill(x, dir)),
        Value::Object(o) => {
            // The worker's exec record has one source, `worker_exec_record` (the GitHub
            // build narrows its `allowed`); the blueprint's own fields win.
            if o.get("id").and_then(Value::as_str) == Some(crate::new_team::WORKER_EXEC) {
                if let Value::Object(rec) = crate::worker_exec::worker_exec_record(dir) {
                    for (k, x) in rec {
                        o.entry(k).or_insert(x);
                    }
                }
            }
            o.values_mut().for_each(|x| fill(x, dir));
        }
        _ => {}
    }
}

/// The blueprint as the text `seed_dir_with` takes, the placeholders of `dir` filled in.
pub fn blueprint_text(dir: &Path) -> Result<String, String> {
    serde_yaml::to_string(&filled(BLUEPRINT, dir)?).map_err(|e| format!("blueprint: {e}"))
}

/// FIRST RUN: write the team's agents into a fresh `dir` after fantastic's stock tree
/// (`fantastic_tui::seed_dir_with`). Returns whether it wrote: `.fantastic` already there
/// means nothing is written.
pub fn seed(dir: &Path) -> Result<bool, String> {
    fantastic_tui::seed_dir_with(dir, &[&blueprint_text(dir)?])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::new_team::{BOARD, GITHUB, LEAD, QUEUE, ROLES, TASKS, WORKER_EXEC, WORKER_FILES};
    use crate::page::{PAGE_ID, WEB_ID, WS_ID};
    use fantastic_kernel::bootstrap::{self, DEFAULT_ROOT_ID};
    use serde_json::{json, Map};
    use std::collections::BTreeMap;

    /// The blueprint's ids with their parents, in order.
    const TREE: [(&str, &str); 11] = [
        (LEAD, "user"),
        (QUEUE, LEAD),
        (GITHUB, LEAD),
        (BOARD, LEAD),
        (ROLES, LEAD),
        (TASKS, LEAD),
        (WORKER_EXEC, LEAD),
        (WORKER_FILES, LEAD),
        (WEB_ID, DEFAULT_ROOT_ID),
        (PAGE_ID, WEB_ID),
        (WS_ID, WEB_ID),
    ];

    /// The agents of `doc` with their parents and records, parents first.
    fn walk<'a>(
        list: &'a Value,
        parent: &'a str,
        out: &mut Vec<(&'a str, &'a str, &'a Map<String, Value>)>,
    ) {
        for e in list.as_array().into_iter().flatten() {
            let o = e.as_object().unwrap();
            let id = o["id"].as_str().unwrap();
            // a parent entry (only `id` and `agents`) names an agent the stock tree wrote
            if o.keys().any(|k| k != "id" && k != "agents") {
                out.push((id, parent, o));
            }
            walk(&e["agents"], id, out);
        }
    }

    /// The team's blueprint for `/team`, as the first run writes it.
    fn declared(dir: &Path) -> Value {
        serde_yaml::from_str(&blueprint_text(dir).unwrap()).unwrap()
    }

    fn rec_in<'a>(doc: &'a Value, id: &str) -> &'a Map<String, Value> {
        let mut out = Vec::new();
        walk(&doc["agents"], DEFAULT_ROOT_ID, &mut out);
        out.into_iter().find(|(i, _, _)| *i == id).unwrap().2
    }

    #[test]
    fn the_blueprint_declares_the_fixed_ids_under_their_parents() {
        let doc = declared(Path::new("/team"));
        let mut out = Vec::new();
        walk(&doc["agents"], DEFAULT_ROOT_ID, &mut out);
        let got: Vec<(&str, &str)> = out.iter().map(|(i, p, _)| (*i, *p)).collect();
        assert_eq!(got, TREE);
        // disk position is the parent, and the stock tree wrote `user`
        assert!(out.iter().all(|(_, _, r)| !r.contains_key("parent_id")));
        assert_eq!(
            doc["agents"][0],
            json!({"id": "user", "agents": doc["agents"][0]["agents"]})
        );
    }

    #[test]
    fn the_blueprint_fills_the_dirs_paths_and_the_workers_exec_record() {
        let dir = Path::new("/team");
        let doc = declared(dir);
        let work = crate::worker_exec::work_dir(dir)
            .to_string_lossy()
            .into_owned();
        assert_eq!(rec_in(&doc, WORKER_FILES)["root"], work.as_str());
        // the exec record is the one `worker_exec_record` makes (the GitHub build patches
        // that one), with the blueprint's grants on top
        let want = crate::worker_exec::worker_exec_record(dir);
        for (k, v) in want.as_object().unwrap() {
            assert_eq!(&rec_in(&doc, WORKER_EXEC)[k], v, "{k}");
        }
        assert_eq!(rec_in(&doc, WORKER_EXEC)["callable_by"]["$parent"], "*");
        // `@root` stays for the seed writer, which resolves it to the kernel's id
        assert!(rec_in(&doc, LEAD)["can_call"].get("@root").is_some());
        // no placeholder is left in the text the seed writes
        assert!(!blueprint_text(dir).unwrap().contains(WORK_PLACEHOLDER));
    }

    /// The page's verb doc has one source in code (`page::READ_STREAM_DOC`) and is written
    /// in the blueprint as it is: the yaml needs no fill for it.
    #[test]
    fn the_blueprints_page_verb_doc_is_the_pages_own() {
        let doc = declared(Path::new("/team"));
        assert_eq!(
            rec_in(&doc, PAGE_ID)["verbs"]["read_stream"],
            crate::page::READ_STREAM_DOC
        );
        assert!(!BLUEPRINT.contains("@read_stream"));
    }

    #[test]
    fn every_declared_module_is_registered() {
        let reg = crate::registry();
        let doc = declared(Path::new("/team"));
        let mut out = Vec::new();
        walk(&doc["agents"], DEFAULT_ROOT_ID, &mut out);
        let mut modules: Vec<String> = out
            .iter()
            .map(|(_, _, r)| r["handler_module"].as_str().unwrap().to_string())
            .collect();
        for t in rec_in(&doc, ROLES)["tiers"].as_object().unwrap().values() {
            modules.push(t["handler_module"].as_str().unwrap().to_string());
        }
        let missing: Vec<&String> = modules.iter().filter(|m| reg.get(m).is_none()).collect();
        assert!(missing.is_empty(), "not registered: {missing:?}");
        assert_eq!(
            rec_in(&doc, WEB_ID)["handler_module"],
            fantastic_web::HANDLER_MODULE
        );
        assert_eq!(
            rec_in(&doc, PAGE_ID)["handler_module"],
            crate::page::HANDLER_MODULE
        );
        assert_eq!(
            rec_in(&doc, WS_ID)["handler_module"],
            fantastic_web_ws::HANDLER_MODULE
        );
    }

    #[test]
    fn every_card_grant_on_a_tool_names_an_agent_the_blueprint_declares() {
        let doc = declared(Path::new("/team"));
        let mut out = Vec::new();
        walk(&doc["agents"], DEFAULT_ROOT_ID, &mut out);
        let ids: Vec<&str> = out.iter().map(|(i, _, _)| *i).collect();
        for (file, text) in crate::roles::ALL {
            let c = team_agentic_resources::cards::parse_card(text, file).unwrap();
            for tool in c.grants.keys() {
                let known = ids.contains(&tool.as_str())
                    || ["$registry", "kernel", "kernel_state", "canvas_page"]
                        .contains(&tool.as_str());
                assert!(
                    known,
                    "{file} grants `{tool}`, which the team does not declare"
                );
            }
        }
    }

    fn on_disk(dir: &Path, id: &str) -> std::path::PathBuf {
        let mut path = vec![id.to_string()];
        let mut at = id;
        while let Some((_, parent)) = TREE.iter().find(|(i, _)| *i == at) {
            if *parent == DEFAULT_ROOT_ID {
                break;
            }
            path.push(parent.to_string());
            at = parent;
        }
        let mut p = dir.join(".fantastic/agents");
        for seg in path.iter().rev() {
            p = p.join(seg).join("agents");
        }
        p.parent().unwrap().join("agent.yaml")
    }

    fn disk_record(dir: &Path, id: &str) -> Value {
        let text = std::fs::read_to_string(on_disk(dir, id))
            .unwrap_or_else(|e| panic!("`{id}` is not on disk: {e}"));
        serde_yaml::from_str(&text).unwrap()
    }

    /// Every agent record under `dir/.fantastic/agents`, by relative path.
    fn tree_of(dir: &Path) -> BTreeMap<String, String> {
        fn go(at: &Path, root: &Path, out: &mut BTreeMap<String, String>) {
            for e in std::fs::read_dir(at).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    go(&p, root, out);
                } else if p.file_name().is_some_and(|n| n == "agent.yaml") {
                    out.insert(
                        p.strip_prefix(root).unwrap().to_string_lossy().into_owned(),
                        std::fs::read_to_string(&p).unwrap(),
                    );
                }
            }
        }
        let root = dir.join(".fantastic/agents");
        let mut out = BTreeMap::new();
        go(&root, &root, &mut out);
        out
    }

    /// A kernel that hydrates `dir`, writes records and serves nothing, as it is after a
    /// boot, then stopped.
    async fn boot_and_stop(dir: &Path) -> BTreeMap<String, Value> {
        let k = crate::start_offline(dir).await.unwrap();
        k.flush_records().await;
        let live: BTreeMap<String, Value> = k
            .agents
            .iter()
            .map(|a| {
                (
                    a.key().0.clone(),
                    Value::Object(a.meta.read().unwrap().clone()),
                )
            })
            .collect();
        k.shutdown().await;
        fantastic_sqlite_store::close_all(&k);
        bootstrap::shutdown(dir).unwrap();
        live
    }

    /// `persist_web` (the team's `webui` word) blocks on a runtime of its own: run it off
    /// the test's runtime.
    fn webui(dir: &Path, port: u16) -> Result<(), String> {
        let dir = dir.to_path_buf();
        std::thread::spawn(move || {
            crate::app_control::persist_web(&dir, Some(port), None).map(|_| ())
        })
        .join()
        .unwrap()
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    /// FIRST RUN: a fresh directory gets every blueprint id on disk, under its parent, as
    /// written, with the grants and the config the yaml says; a directory that has a
    /// kernel is not seeded again.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_first_run_seeds_the_fixed_ids_with_their_grants_and_config_from_the_yaml() {
        let _web = crate::WEB_LOCK.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        assert!(seed(dir.path()).unwrap());
        let doc = declared(dir.path());
        for (id, _) in TREE {
            let on = disk_record(dir.path(), id);
            assert_eq!(on["id"], id, "{id}");
            assert!(on.get("parent_id").is_none(), "{id}");
            // every field the yaml declares is on disk as written (`@root` resolved)
            for (k, v) in rec_in(&doc, id)
                .iter()
                .filter(|(k, _)| !["id", "agents"].contains(&k.as_str()))
            {
                let mut want = v.clone();
                if ["can_call", "callable_by"].contains(&k.as_str()) {
                    let g = want.as_object_mut().unwrap();
                    if let Some(x) = g.remove("@root") {
                        g.insert(DEFAULT_ROOT_ID.into(), x);
                    }
                }
                assert_eq!(on[k], want, "{id}.{k}");
            }
        }
        // the grants and the config, by name
        let lead = disk_record(dir.path(), LEAD);
        assert_eq!(lead["memory_store_id"], "memory");
        assert_eq!(
            lead["can_call"][DEFAULT_ROOT_ID],
            json!(crate::new_team::LEAD_ROOT_VERBS)
        );
        assert_eq!(
            lead["can_call"]["roles"],
            json!(crate::new_team::LEAD_ROLES_VERBS)
        );
        assert_eq!(disk_record(dir.path(), BOARD)["github"], GITHUB);
        assert_eq!(disk_record(dir.path(), QUEUE)["callable_by"]["board"], "*");
        assert_eq!(
            disk_record(dir.path(), WEB_ID)["can_call"][PAGE_ID],
            json!(["read_stream"])
        );
        assert_eq!(
            disk_record(dir.path(), WS_ID)["callable_by"][WEB_ID],
            json!(["get_routes"])
        );
        // the stock tree is there too, and `user` is the one the stock blueprint wrote
        assert_eq!(
            disk_record(dir.path(), "user")["handler_module"],
            "user.tools"
        );
        // no second seed
        assert!(!seed(dir.path()).unwrap());
        // each is live under that id at the boot
        let live = boot_and_stop(dir.path()).await;
        for (id, _) in TREE {
            assert!(live.contains_key(id), "{id} is live");
        }
    }

    /// A SECOND BOOT changes nothing: the records of the team and of fantastic's stock tree
    /// are the same bytes after the first boot as after the second.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_second_boot_changes_nothing() {
        let _web = crate::WEB_LOCK.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        seed(dir.path()).unwrap();
        let first = boot_and_stop(dir.path()).await;
        let on_first = tree_of(dir.path());
        let second = boot_and_stop(dir.path()).await;
        assert_eq!(tree_of(dir.path()), on_first, "the records on disk");
        // the kernel's own id counter moves with every boot; nothing else does
        let steady = |mut m: BTreeMap<String, Value>| {
            m.get_mut(DEFAULT_ROOT_ID)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove("next_id");
            m
        };
        assert_eq!(steady(second), steady(first), "the agents as they are live");
        // and the `webui` word, run twice with the same port, adds nothing after the first
        let port = free_port();
        webui(dir.path(), port).unwrap();
        let after = tree_of(dir.path());
        webui(dir.path(), port).unwrap();
        assert_eq!(tree_of(dir.path()), after);
    }

    /// A fixed agent deleted from disk stays deleted at the next boot: the blueprint is not
    /// read again. This is what fantastic does for its own stock agents (`src/tui/src/boot/
    /// seed.rs:3-6`, `seed_dir_with` at :188 writes only into a folder that is fresh), and
    /// is shown here on both, a team agent and a stock one, through the team's `webui` word
    /// (the entry that used to `ensure`) and a plain boot.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_deleted_fixed_agent_stays_deleted_like_fantastics_own_stock_agents() {
        let _web = crate::WEB_LOCK.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        seed(dir.path()).unwrap();
        let port = free_port();
        webui(dir.path(), port).unwrap();
        let stock = dir.path().join(".fantastic/agents/user/agents/exec");
        assert!(stock.is_dir());
        std::fs::remove_dir_all(on_disk(dir.path(), BOARD).parent().unwrap()).unwrap();
        std::fs::remove_dir_all(&stock).unwrap();

        webui(dir.path(), port).unwrap();
        let live = boot_and_stop(dir.path()).await;
        assert!(
            !live.contains_key(BOARD),
            "the team's board is not made again"
        );
        assert!(!live.contains_key("exec"), "nor fantastic's stock exec");
        assert!(!on_disk(dir.path(), BOARD).exists());
        assert!(!stock.exists());
        // the rest of the team is as it was
        for id in [LEAD, QUEUE, GITHUB, ROLES, TASKS, WEB_ID, PAGE_ID, WS_ID] {
            assert!(live.contains_key(id), "{id}");
        }
    }

    /// A field changed on disk stays as it is at the next boot (the blueprint is not read
    /// again): a team agent's declared field, the same as a stock agent's.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_drifted_field_stays_as_it_is_like_fantastics_own_stock_agents() {
        let _web = crate::WEB_LOCK.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        seed(dir.path()).unwrap();
        let port = free_port();
        webui(dir.path(), port).unwrap();
        // a declared grant of the queue, a declared view-ish field of the board, and a stock
        // agent's declared field
        let edit = |id_path: std::path::PathBuf, f: &dyn Fn(&mut Value)| {
            let mut rec: Value =
                serde_yaml::from_str(&std::fs::read_to_string(&id_path).unwrap()).unwrap();
            f(&mut rec);
            std::fs::write(id_path, serde_yaml::to_string(&rec).unwrap()).unwrap();
        };
        edit(on_disk(dir.path(), QUEUE), &|r| {
            r["callable_by"] = json!({"$self": "*", "extra": ["read"]});
            r["display_name"] = json!("changed");
        });
        edit(on_disk(dir.path(), BOARD), &|r| {
            r["repos"] = json!(["me/mine"])
        });
        let files = dir
            .path()
            .join(".fantastic/agents/user/agents/files/agent.yaml");
        edit(files, &|r| r["allow"] = json!(["docs"]));

        webui(dir.path(), port).unwrap();
        let live = boot_and_stop(dir.path()).await;
        let q = &live[QUEUE];
        assert_eq!(q["display_name"], "changed", "{q}");
        assert!(
            q["callable_by"].get("board").is_none(),
            "the declared grant is not put back"
        );
        assert_eq!(q["callable_by"]["extra"], json!(["read"]));
        assert_eq!(live[BOARD]["repos"], json!(["me/mine"]));
        assert_eq!(
            live["files"]["allow"],
            json!(["docs"]),
            "a stock agent's field too"
        );
    }
}
