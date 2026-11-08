//! TEAM NOTICE: when an AI agent is added under a lead, or removed from it, that lead
//! and every AI agent in its subtree get the team's structure (who leads, every member
//! with its parent and role). A spawned agent's task comes from the roles registry, so
//! without it a member cannot tell whom it reports to. Only that lead's subtree is told:
//! the structure of a team is its lead's business, nobody else's.
//!
//! The cadencer runs it: one tap on the kernel's state stream (`added` / `removed`),
//! a short debounce so a burst of spawns is one notice, and the subtree from
//! fantastic's own [`fantastic_kernel::reflect::descendant_ids`] (no tree walk here).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use fantastic_kernel::{AgentId, Item, Kernel};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// A burst of adds and removes within this is one notice per lead.
pub const DEBOUNCE: Duration = Duration::from_millis(1000);

/// One change seen on the state stream: `added` (with its parent) or `removed`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Change {
    Added { id: String, parent: String },
    Removed { id: String },
}

/// An AI agent: an AI chat, the agent that names its provider (`provider_id`). A provider
/// keeps a memory store too, but holds no conversation and takes no message.
fn is_ai(kernel: &Kernel, id: &str) -> bool {
    kernel
        .agents
        .get(&AgentId::from(id))
        .is_some_and(|a| a.value().record().meta.contains_key(fantastic_ai_core::remote_provider::PROVIDER_FIELD))
}

/// The leads whose team changed in `batch`. `parents` remembers each AI member's lead,
/// so a removal (its event names no parent) still finds it. A non-AI agent (a tool)
/// changes no team.
pub(crate) fn leads_of(
    batch: Vec<Change>,
    parents: &mut BTreeMap<String, String>,
    is_ai: impl Fn(&str) -> bool,
) -> BTreeSet<String> {
    let mut leads = BTreeSet::new();
    for c in batch {
        match c {
            Change::Added { id, parent } => {
                if is_ai(&id) {
                    parents.insert(id, parent.clone());
                    leads.insert(parent);
                }
            }
            Change::Removed { id } => {
                if let Some(p) = parents.remove(&id) {
                    leads.insert(p);
                }
            }
        }
    }
    leads
}

/// A member of a team: its id, its parent and its role (empty when it has none).
pub(crate) type Member = (String, String, String);

/// The AI agents in `lead`'s subtree (the lead itself not included).
pub(crate) fn members(kernel: &Kernel, lead: &str) -> Vec<Member> {
    let Some(a) = kernel.agents.get(&AgentId::from(lead)) else {
        return Vec::new();
    };
    let ids = fantastic_kernel::reflect::descendant_ids(a.value());
    drop(a);
    ids.into_iter()
        .skip(1)
        .filter(|id| is_ai(kernel, id))
        .filter_map(|id| {
            let a = kernel.agents.get(&AgentId::from(id.as_str()))?;
            let parent = a.value().parent_id.as_ref().map(|p| p.0.clone()).unwrap_or_default();
            let role = a
                .value()
                .record()
                .meta
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            Some((id, parent, role))
        })
        .collect()
}

/// The notice text: who leads the team, and each member with its parent and role.
pub(crate) fn message(lead: &str, members: &[Member], why: &str) -> String {
    let mut s = format!(
        "[team] The team of `{lead}` {why}. `{lead}` leads it. Report to your parent (below) \
         with `send` when you are done.\nMembers:"
    );
    if members.is_empty() {
        s.push_str(" none");
    }
    for (id, parent, role) in members {
        let role = if role.is_empty() { String::new() } else { format!(" ({role})") };
        s.push_str(&format!("\n- `{id}`{role}, parent `{parent}`"));
    }
    s
}

/// Who gets `lead`'s notice and what it says: the lead (when it is an AI) and every AI
/// member of its subtree, one text for all. Empty when there is nobody to tell.
pub(crate) fn plan(kernel: &Kernel, lead: &str, why: &str) -> (Vec<String>, String) {
    if !kernel.agents.contains_key(&AgentId::from(lead)) {
        return (Vec::new(), String::new());
    }
    let ms = members(kernel, lead);
    let mut to: Vec<String> = ms.iter().map(|m| m.0.clone()).collect();
    if is_ai(kernel, lead) {
        to.insert(0, lead.to_string());
    }
    (to, message(lead, &ms, why))
}

async fn notify(kernel: &Arc<Kernel>, lead: &str, why: &str) {
    let (to, text) = plan(kernel, lead, why);
    if to.is_empty() {
        return;
    }
    crate::tree::grant_digest_targets(kernel, &to);
    for id in to {
        let _ = kernel
            .send(vec![Item::new(AgentId::from(id.as_str()), json!({"type": "send", "text": text}))])
            .await;
    }
}

/// Tap the state stream and send each changed team its notice, debounced. Returns the
/// task; the caller keeps it so the tap is started once per cadencer.
pub fn start(kernel: &Arc<Kernel>) -> JoinHandle<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Change>();
    // The tap only queues: the kernel is not touched from inside its own publish.
    kernel.add_state_subscriber(Arc::new(move |ev: &Value, _body| {
        let id = ev.get("agent_id").and_then(Value::as_str).unwrap_or("").to_string();
        let change = match ev.get("kind").and_then(Value::as_str) {
            Some("added") => ev
                .get("parent_id")
                .and_then(Value::as_str)
                .map(|p| Change::Added { id, parent: p.to_string() }),
            Some("removed") => Some(Change::Removed { id }),
            _ => None,
        };
        if let Some(c) = change {
            let _ = tx.send(c);
        }
    }));
    let weak = Arc::downgrade(kernel);
    tokio::spawn(async move {
        // the AI members that exist already, so their removal is seen too
        let mut parents: BTreeMap<String, String> = match weak.upgrade() {
            Some(k) => k
                .agents
                .iter()
                .filter(|e| e.value().record().meta.contains_key(fantastic_ai_core::remote_provider::PROVIDER_FIELD))
                .filter_map(|e| Some((e.key().0.clone(), e.value().parent_id.as_ref()?.0.clone())))
                .collect(),
            None => return,
        };
        // BOOTED UP: every team that exists now is told its structure once, so a team made
        // before the cadencer (or a restart) knows whom it reports to
        if let Some(k) = weak.upgrade() {
            let leads: BTreeSet<String> = parents.values().cloned().collect();
            for lead in leads.into_iter().filter(|l| is_ai(&k, l)) {
                notify(&k, &lead, "booted up").await;
            }
        }
        while let Some(first) = rx.recv().await {
            tokio::time::sleep(DEBOUNCE).await;
            let mut batch = vec![first];
            while let Ok(c) = rx.try_recv() {
                batch.push(c);
            }
            let Some(k) = weak.upgrade() else { return };
            // a team is an AI lead and its subtree: a change under a non-AI agent (the user) tells nobody
            for lead in leads_of(batch, &mut parents, |id| is_ai(&k, id)).into_iter().filter(|l| is_ai(&k, l)) {
                notify(&k, &lead, "changed").await;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fantastic_kernel::bootstrap::DEFAULT_ROOT_ID;

    #[test]
    fn leads_of_takes_ai_adds_and_known_removes_only() {
        let mut parents = BTreeMap::new();
        let ai = |id: &str| id.starts_with("ai");
        let leads = leads_of(
            vec![
                Change::Added { id: "ai1".into(), parent: "L".into() },
                Change::Added { id: "tool".into(), parent: "L2".into() },
                Change::Added { id: "ai2".into(), parent: "M".into() },
            ],
            &mut parents,
            ai,
        );
        assert_eq!(leads, BTreeSet::from(["L".to_string(), "M".to_string()]), "a tool changes no team");
        let leads = leads_of(
            vec![Change::Removed { id: "ai1".into() }, Change::Removed { id: "unknown".into() }],
            &mut parents,
            ai,
        );
        assert_eq!(leads, BTreeSet::from(["L".to_string()]));
        assert!(!parents.contains_key("ai1"));
    }

    #[test]
    fn message_names_the_lead_and_each_member_with_parent_and_role() {
        let m = message(
            "lead",
            &[("w1".into(), "lead".into(), "worker".into()), ("w2".into(), "w1".into(), String::new())],
            "changed",
        );
        assert!(m.starts_with("[team] The team of `lead` changed. `lead` leads it."), "{m}");
        assert!(m.contains("\n- `w1` (worker), parent `lead`"), "{m}");
        assert!(m.contains("\n- `w2`, parent `w1`"), "{m}");
        assert!(message("x", &[], "booted up").starts_with("[team] The team of `x` booted up."));
        assert!(message("x", &[], "changed").ends_with("Members: none"));
    }

    /// Records: `L` (AI) → `a` (AI, worker), `t` (a tool) ; `a` → `c` (AI) ; and apart, `M` (AI) → `x` (AI).
    async fn tree() -> (tempfile::TempDir, Arc<Kernel>) {
        let dir = tempfile::tempdir().unwrap();
        let kernel = crate::start(dir.path()).await.unwrap();
        for (id, parent, ai, role) in [
            ("L", DEFAULT_ROOT_ID, true, ""),
            ("a", "L", true, "worker"),
            ("t", "L", false, ""),
            ("c", "a", true, ""),
            ("M", DEFAULT_ROOT_ID, true, ""),
            ("x", "M", true, ""),
        ] {
            let mut mk = json!({"type": "create_agent", "id": id, "parent_id": parent, "handler_module": "ram_store.tools",
                "callable_by": {"$self": "*", "$parent": "*"}});
            if ai {
                mk["provider_id"] = json!("p");
            }
            if !role.is_empty() {
                mk["role"] = json!(role);
            }
            let r = kernel.send(vec![Item::new(DEFAULT_ROOT_ID, mk)]).await.remove(0).result;
            assert!(r.get("error").is_none(), "{id}: {r}");
        }
        (dir, kernel)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn plan_tells_the_lead_and_its_ai_subtree_only() {
        let (_dir, k) = tree().await;
        let (to, text) = plan(&k, "L", "changed");
        assert_eq!(to, ["L", "a", "c"], "the lead, then its AI members; not the tool, not M's team");
        assert!(text.contains("`a` (worker), parent `L`") && text.contains("`c`, parent `a`"), "{text}");
        assert!(!text.contains("`x`") && !text.contains("`t`"), "{text}");
        let (to, _) = plan(&k, "gone", "changed");
        assert!(to.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_ai_added_under_a_lead_makes_the_tap_tell_that_team_only() {
        let (_dir, k) = tree().await;
        let mk = json!({"type": "create_agent", "id": crate::tree::CADENCER_ID, "parent_id": DEFAULT_ROOT_ID,
            "handler_module": "ram_store.tools", "callable_by": {"$self": "*", "$parent": "*"}});
        k.send(vec![Item::new(DEFAULT_ROOT_ID, mk)]).await;
        let _tap = start(&k);
        let mk = json!({"type": "create_agent", "id": "b", "parent_id": "L", "handler_module": "ram_store.tools",
            "provider_id": "p", "callable_by": {"$self": "*", "$parent": "*"}});
        k.send(vec![Item::new(DEFAULT_ROOT_ID, mk)]).await;
        // the notice grants the cadencer `send` on every member it tells, then sends
        let told = |id: &str| {
            k.agents.get(&AgentId::from(id)).is_some_and(|a| {
                a.value().record().meta.get("callable_by").and_then(|c| c.get(crate::tree::CADENCER_ID)).is_some()
            })
        };
        for _ in 0..60 {
            if told("b") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        // (M's team is told at boot; that a change tells only the changed team is `plan`'s test)
        assert!(told("L") && told("a") && told("b") && told("c"), "L's team, the new member included, is told");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn at_boot_every_ai_led_team_is_told_once() {
        let (_dir, k) = tree().await;
        let mk = json!({"type": "create_agent", "id": crate::tree::CADENCER_ID, "parent_id": DEFAULT_ROOT_ID,
            "handler_module": "ram_store.tools", "callable_by": {"$self": "*", "$parent": "*"}});
        k.send(vec![Item::new(DEFAULT_ROOT_ID, mk)]).await;
        let _tap = start(&k);
        let told = |id: &str| {
            k.agents.get(&AgentId::from(id)).is_some_and(|a| {
                a.value().record().meta.get("callable_by").and_then(|c| c.get(crate::tree::CADENCER_ID)).is_some()
            })
        };
        for _ in 0..60 {
            if told("L") && told("M") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(["L", "a", "c", "M", "x"].iter().all(|i| told(i)), "both AI-led teams are told at boot");
        assert!(!told("t"), "a tool is told nothing");
    }
}
