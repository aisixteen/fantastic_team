//! The fields a task registry's record carries, beside the ones every record has.

use schemars::JsonSchema;

/// A task registry's own record fields.
#[derive(Debug, JsonSchema)]
pub struct Record {
    /// The store agent that keeps the task tables; absent, the registry refuses every verb `disabled`.
    #[schemars(length(min = 1))]
    pub task_store_id: Option<String>,
}

fantastic_kernel::record!(Record);
