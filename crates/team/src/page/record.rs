//! The fields the team page's record carries, beside the ones every record has.

use schemars::JsonSchema;
use serde_json::Value;

/// The team page's own record field.
#[derive(Debug, JsonSchema)]
pub struct Record {
    /// The source verbs the page declares to its `web` host (`read_stream`, as [`crate::page::read_stream_declared`] declares it); the host serves an agent that declares `read_stream`.
    pub verbs: Option<Value>,
}

fantastic_kernel::record!(Record);
