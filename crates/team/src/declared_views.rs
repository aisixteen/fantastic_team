//! The team page's html views: which agents' own `reflect` carries the view the page
//! mounts ([`page_views`]), and the one grant that lets the page's leg read it and make
//! the view's calls ([`grant_views`]).

use std::sync::Arc;

use fantastic_kernel::{AgentId, Kernel};

/// The html views the team page draws (each agent's `reflect` carries its view): the
/// handler module and the verbs that view calls on its agent.
pub fn page_views() -> [(&'static str, &'static [&'static str]); 4] {
    [
        (
            crate::cadencer::HANDLER_MODULE,
            &crate::cadencer::VIEW_CALLS,
        ),
        (
            team_github_connector::HANDLER_MODULE,
            &team_github_connector::VIEW_CALLS,
        ),
        (
            team_agentic_resources::HANDLER_MODULE,
            &team_agentic_resources::VIEW_CALLS,
        ),
        (
            team_task_registry::HANDLER_MODULE,
            &team_task_registry::VIEW_CALLS,
        ),
    ]
}

/// What the page leg needs on an agent to draw its html view: `reflect`, which the page
/// reads the view from, then the view's own calls.
pub fn page_verbs(calls: &[&'static str]) -> Vec<&'static str> {
    let mut v = vec!["reflect"];
    v.extend(calls.iter().filter(|c| **c != "reflect"));
    v
}

/// Grant the team page's leg `reflect` plus the calls of each html view ([`page_views`]),
/// on both ends through [`crate::tree::grant_to`]: never `*`, verbs another grant put there
/// stay. Idempotent. Returns how many agents got a grant written.
pub fn grant_views(kernel: &Arc<Kernel>) -> usize {
    let mut written = 0;
    let mut page: Vec<(AgentId, Vec<&str>)> = Vec::new();
    for e in kernel.agents.iter() {
        let a = e.value();
        let Some(m) = a.handler_module.as_deref() else {
            continue;
        };
        if let Some((_, calls)) = page_views().iter().find(|(h, _)| *h == m) {
            page.push((a.id.clone(), page_verbs(calls)));
        }
    }
    page.sort_by(|a, b| a.0 .0.cmp(&b.0 .0));
    for (id, verbs) in &page {
        written += usize::from(crate::tree::grant_to(
            kernel,
            crate::page::WS_ID,
            id.as_str(),
            verbs,
        ));
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_boot::{as_caller, boot_at, Seed};
    use fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;
    use fantastic_kernel::Item;
    use serde_json::{json, Value};

    // ---- records: grant_views ----

    async fn mk(kernel: &Arc<Kernel>, rec: Value) {
        let mut rec = rec;
        rec["type"] = json!("create_agent");
        rec["parent_id"] = json!(DEFAULT_ROOT_ID);
        let out = kernel.send(vec![Item::new(DEFAULT_ROOT_ID, rec)]).await;
        assert!(out[0].result.get("error").is_none(), "{}", out[0].result);
    }

    fn field_of(kernel: &Arc<Kernel>, id: &str, f: &str) -> Value {
        kernel
            .agents
            .get(&AgentId::from(id))
            .map(|a| {
                a.meta
                    .read()
                    .unwrap()
                    .get(f)
                    .cloned()
                    .unwrap_or(Value::Null)
            })
            .unwrap_or(Value::Null)
    }

    #[test]
    fn page_verbs_lead_with_reflect_once() {
        assert_eq!(
            page_verbs(&["status", "set"]),
            vec!["reflect", "status", "set"]
        );
        assert_eq!(
            page_verbs(&["reflect", "pr_status"]),
            vec!["reflect", "pr_status"]
        );
        assert_eq!(page_verbs(&[]), vec!["reflect"]);
    }

    /// The team page reads a view with `reflect` AS ITS LEG (`team.html`: `send(id, {type:
    /// 'reflect'})` over the `team_ws` socket), so the leg needs `reflect` and the view's calls
    /// on each agent whose view it draws, and nothing more. The reflect here is sent as the
    /// leg, not as the operator: the gate is the one the page meets.
    #[tokio::test(flavor = "multi_thread")]
    async fn grant_views_lets_the_page_leg_reflect_each_html_view_and_call_it_and_no_more() {
        let dir = tempfile::tempdir().unwrap();
        let kernel = boot_at(dir.path(), Seed::Stock, &[], None).await;
        let leg = crate::page::WS_ID;
        mk(
            &kernel,
            json!({"id": "cad", "handler_module": crate::cadencer::HANDLER_MODULE}),
        )
        .await;
        mk(
            &kernel,
            json!({"id": "gh", "handler_module": team_github_connector::HANDLER_MODULE,
                "app": {"credentials": {"store": "credentials", "ref": "github/app"}},
                "repos": ["o/r"], "directive": "Push a work branch."}),
        )
        .await;
        mk(
            &kernel,
            json!({"id": "res", "handler_module": team_agentic_resources::HANDLER_MODULE}),
        )
        .await;
        mk(
            &kernel,
            json!({"id": "tasks", "handler_module": team_task_registry::HANDLER_MODULE}),
        )
        .await;
        // the leg comes last: the cadencer's boot (which grants the views) found no leg yet
        mk(
            &kernel,
            json!({"id": leg, "handler_module": "ram_store.tools"}),
        )
        .await;
        let views: [(&str, &[&str]); 4] = [
            ("cad", &crate::cadencer::VIEW_CALLS),
            ("gh", &team_github_connector::VIEW_CALLS),
            ("res", &team_agentic_resources::VIEW_CALLS),
            ("tasks", &team_task_registry::VIEW_CALLS),
        ];
        // before the grant the leg's reflect is refused by the gate: the view never mounts
        for (id, _) in views {
            let r = as_caller(&kernel, leg, id, json!({"type": "reflect"})).await;
            assert_eq!(r["reason"], "unauthorized", "{id}: {r}");
        }
        assert!(grant_views(&kernel) > 0);
        let cc = field_of(&kernel, leg, "can_call");
        assert_eq!(
            cc["cad"],
            json!(["reflect", "watch", "status", "set"]),
            "{cc}"
        );
        assert_eq!(cc["gh"], json!(["reflect", "watch", "pr_status"]), "{cc}");
        assert_eq!(
            cc["tasks"],
            json!(["reflect", "watch", "list", "get"]),
            "{cc}"
        );
        for (id, calls) in views {
            let want = json!(page_verbs(calls));
            assert_eq!(cc[id], want, "{cc}");
            assert_eq!(field_of(&kernel, id, "callable_by")[leg], want, "{id}");
            // no spawn, no star
            let v = cc[id].to_string();
            assert!(!v.contains("spawn") && !v.contains('*'), "{id}: {v}");
            // sent as the leg, the reflect is admitted and carries the view, whose calls are
            // the verbs granted
            let r = as_caller(&kernel, leg, id, json!({"type": "reflect"})).await;
            assert_eq!(r["view"]["kind"], "html", "{id}: {r}");
            assert_eq!(r["view"]["calls"][id], json!(calls), "{id}: {r}");
            // and the view's own calls pass the gate as the leg too
            let last = calls[calls.len() - 1];
            let c = as_caller(&kernel, leg, id, json!({"type": last})).await;
            assert_ne!(c["reason"], "unauthorized", "{id} {last}: {c}");
        }
        // an agent without an html view gets nothing
        assert!(cc.get("dispatcher").is_none(), "{cc}");
        // Idempotent.
        assert_eq!(grant_views(&kernel), 0);
        kernel.stop().await;
    }
}
