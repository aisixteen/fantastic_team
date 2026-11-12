//! The fields a worker view's record carries beside the ones every record has: none.

use schemars::JsonSchema;

/// A worker view's own record fields: none.
#[derive(Debug, JsonSchema)]
pub struct Record {}

fantastic_kernel::record!(Record);
