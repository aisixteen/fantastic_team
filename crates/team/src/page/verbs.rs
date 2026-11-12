//! The verbs the team page declares: one struct per verb, then `verbs!` — the list the
//! agent's own entry checks every call against and `reflect` and `docs` render.

use schemars::JsonSchema;
use serde::Deserialize;

/// The served paths. Returns {files}.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct List {}

/// A file of the team page as a BINARY reply: the bytes as the body, {path, offset, next_offset, eof, size} as the head.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadStream {
    /// The served file's path, as the page names it (no leading slash).
    #[schemars(length(min = 1))]
    pub path: String,
    /// Where the chunk starts, in bytes.
    #[serde(default)]
    pub offset: u64,
    /// The chunk size in bytes; absent, the rest of the file.
    pub length: Option<u64>,
}

fantastic_kernel::verbs! {
    "list" => List,
    "read_stream" => ReadStream,
}
