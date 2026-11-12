//! The verbs the bundles registry declares: one struct per verb, then `verbs!` — the list
//! the agent's own entry checks every call against and `reflect` and `docs` render.

use schemars::JsonSchema;
use serde::Deserialize;

/// Every spawnable bundle: each meta bundle (bundles/<name>/bundle.yaml) whose record carries a directive; `reflect` with bundles:'all' lists the same bundles. Read on each call. Returns {bundles:[{name, purpose}], errors:[...]}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bundles {}

/// One spawnable bundle. Returns {name, purpose, grants, directive}; an unknown name is `not_found`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    /// The bundle's name.
    #[schemars(length(min = 1))]
    pub name: String,
}

/// A fork point of an existing agent.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ForkFrom {
    /// The agent to branch off.
    #[schemars(length(min = 1))]
    pub agent: String,
    /// The fork point (a row of its conversation).
    pub at: i64,
}

const SPAWN_DOC: &str = "Creates an agent in one of two shapes. {bundle, name?, model?, provider?} creates it from the bundle (its directive after the default system block, its grants), an AI chat completing through `provider` on `model`, as a CHILD OF THE CALLER, gives the caller pause/resume/stop on it, and lets the child `send` the caller (both ends, merged with what either already holds), so it tells its parent with a message when it is done. Spawn sends nothing and takes no task: AFTER SPAWNING, send each child its work yourself (`send` with `text`), telling it you are its parent (your id, the one it reports to with `send` when done) and who its siblings are. {from:{agent, at}} makes a CLONE of `agent` at a fork point (ai_core `fork {at}`, same memory store, so it starts with that conversation up to `at`) and takes no other field: the clone keeps every setting of the chat (its bundle is the source's own, and its directive, model, name and provider are the source's, none is applied or changed), it sits BESIDE its source, not under you, and you still get pause/resume/stop on it and it may `send` you; a call giving `from` with `bundle` is refused; you must hold `fork` on `agent` or it is refused with reason `not_allowed`; a refusal from fork (cuts_turn, cuts_fold, no_such_row, parent_mid_turn, ...) passes through unchanged and nothing is left behind. Then the reply also has from:{agent, at} and copied. The bundle's grants on tools are opened on both ends (the child's own grant and the tool's callable_by, all or nothing): a bundle may only grant what the spawner itself holds, or the spawn is refused with reason `not_allowed`, and a grant that cannot be written undoes the whole spawn (reason `grant_failed`). Returns {id, bundle, provider, model}.";

/// A new agent from a bundle.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fresh {
    /// The bundle to spawn.
    pub bundle: String,
    /// The provider agent the child completes through; yours when omitted.
    pub provider: Option<String>,
    /// The model it runs there; yours when omitted, else the provider's default.
    pub model: Option<String>,
    /// The child's display name.
    pub name: Option<String>,
}

/// A clone of an existing agent at a fork point.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cloned {
    /// The agent to branch off, and the row of its conversation.
    pub from: ForkFrom,
}

/// The two shapes of `spawn`: a new agent from a bundle, or a clone from a fork point.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Spawn {
    /// `{bundle, name?, model?, provider?}`.
    Fresh(Fresh),
    /// `{from:{agent, at}}`.
    Cloned(Cloned),
}

/// `spawn`'s declaration: the fields of both shapes and the engine's `oneOf` of
/// `required` lists, so a call naming both `bundle` and `from`, or neither, is refused.
impl JsonSchema for Spawn {
    fn schema_name() -> String {
        "Spawn".to_string()
    }

    fn json_schema(gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        let from = serde_json::to_value(gen.subschema_for::<ForkFrom>()).expect("a from schema");
        serde_json::from_value(serde_json::json!({
            "type": "object",
            "description": SPAWN_DOC,
            "properties": {
                "bundle": {"type": "string", "minLength": 1, "description": "The bundle to spawn."},
                "provider": {"type": "string", "minLength": 1, "description": "The provider agent the child completes through; yours when omitted."},
                "model": {"type": "string", "minLength": 1, "description": "The model it runs there; yours when omitted, else the provider's default."},
                "name": {"type": "string", "minLength": 1, "description": "The child's display name."},
                "from": from,
            },
            "oneOf": [{"required": ["bundle"]}, {"required": ["from"]}],
            "additionalProperties": false,
        }))
        .expect("a spawn schema")
    }
}

/// The agents running on a spawnable bundle, per bundle, read from the tree (survives restart). Returns {bundles:{<bundle>:[{id}]}}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fleet {}

/// How many agents run on spawnable bundles, read from the tree. Returns {running}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCount {
    /// The bundle to count; omitted = all spawnable bundles together.
    #[schemars(length(min = 1))]
    pub bundle: Option<String>,
}

fantastic_kernel::verbs! {
    "bundles" => Bundles,
    "bundle" => Bundle,
    "spawn" => Spawn,
    "fleet" => Fleet,
    "get_count" => GetCount,
}
