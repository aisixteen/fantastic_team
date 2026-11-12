//! The fields a git connector's record carries, beside the ones every record has.

use fantastic_ai_core::record::Credentials;
use schemars::JsonSchema;

/// A git connector's own record fields.
#[derive(Debug, JsonSchema)]
pub struct Record {
    /// Absolute path of the local clone every worktree is made from.
    pub repo_dir: String,
    /// Absolute directory every worktree path must be inside; absent = <the default folder>/<agent id>, made at the first worktree_add (the default folder is the system temp folder's fantastic_team/worktrees). Set or change it (update_agent) only after the user agrees to that folder: ask first.
    pub workspace_root: Option<String>,
    /// A remote name in repo_dir (origin) or a URL (https://…, ssh://…, git@host:org/repo.git).
    pub remote: String,
    /// The remote's default branch — push refuses it (and main, master).
    pub default_branch: String,
    /// The credential store ref of an HTTPS token (used for HTTPS only).
    pub credentials: Option<Credentials>,
    /// The HTTPS user name sent with the token (GitHub: x-access-token, GitLab: oauth2).
    #[serde(default = "username")]
    pub username: String,
    /// Commit author and sign-off name (with author_email).
    pub author_name: Option<String>,
    /// Commit author and sign-off email (with author_name).
    pub author_email: Option<String>,
    /// GIT_SSH_COMMAND for SSH remotes.
    #[serde(default = "ssh_command")]
    pub ssh_command: String,
    /// Seconds one git command may run.
    #[serde(default = "timeout_s")]
    #[schemars(range(min = 1))]
    pub timeout_s: u64,
}

fn username() -> String {
    "x-access-token".to_string()
}

fn ssh_command() -> String {
    "ssh -o BatchMode=yes".to_string()
}

fn timeout_s() -> u64 {
    300
}

fantastic_kernel::record!(Record);
