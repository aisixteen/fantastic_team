//! Roles are META BUNDLES (fantastic's `bundles/<name>/bundle.yaml` with a `base` and no
//! program): a role is a bundle whose `record` carries a `post_system_prompt` (its
//! directive) and a `can_call` (what it may call). The registry reads them from the
//! kernel's bundles on every call; there is no card file and no folder of its own.

use fantastic_kernel::Kernel;
use serde_json::{Map, Value};

/// A grant key that stands for the registry's own id (a bundle may also name it).
pub const REGISTRY_TOKEN: &str = "$registry";

/// One role: a meta bundle that is an AI agent's directive.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    /// The role's name: the bundle's.
    pub name: String,
    /// One line: what the role is for (the bundle's `sentence`).
    pub purpose: String,
    /// What the child may call (the bundle's `record.can_call`, `$self` aside).
    pub grants: Map<String, Value>,
    /// Who may call the child (the bundle's `record.callable_by`), beside its spawner and
    /// the registry: a lead's `$subtree: [send]` lets the scheduler it makes wake it.
    pub callable_by: Map<String, Value>,
    /// The directive (the bundle's `record.post_system_prompt`).
    pub directive: String,
}

impl Card {
    /// The role as the `role` verb answers it.
    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "name": self.name, "purpose": self.purpose,
            "grants": Value::Object(self.grants.clone()), "directive": self.directive,
        })
    }
}

/// Every role the kernel's bundles hold, by name: each loaded meta bundle whose fields
/// carry a directive.
#[must_use]
pub fn of(kernel: &Kernel) -> Vec<Card> {
    let mut out: Vec<Card> = kernel
        .bundles
        .folder_rows()
        .iter()
        .filter(|r| r.get("error").is_none() && r.get("base").is_some())
        .filter_map(|r| {
            let name = r.get("name")?.as_str()?.to_string();
            if kernel.bundles.meta_chain(&name).is_empty() {
                return None;
            }
            let fields = kernel.bundles.defaults(&name);
            let directive = fields.get("post_system_prompt")?.as_str()?.to_string();
            let mut grants = fields
                .get("can_call")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            grants.remove("$self");
            let callable_by = fields
                .get("callable_by")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            Some(Card {
                callable_by,
                purpose: r.get("sentence").and_then(Value::as_str).unwrap_or("").to_string(),
                name,
                grants,
                directive,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}
