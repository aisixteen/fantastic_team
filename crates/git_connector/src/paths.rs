//! Path rules: every worktree path stays inside `workspace_root`, and a
//! verb that works on a worktree first checks that the path really is one of
//! `repo_dir`'s worktrees.

use std::path::{Component, Path, PathBuf};

use serde_json::{json, Value};

use crate::git;
use crate::{bad, Cfg};

/// Refusal: the path is outside the workspace root.
pub(crate) fn outside(verb: &str, path: &str, root: &Path) -> Value {
    bad(
        format!(
            "team_git {verb}: path `{path}` is outside the workspace root `{}`",
            root.display()
        ),
        "path",
        json!(path),
    )
}

/// Refusal: the path is not a worktree of `repo_dir`.
pub(crate) fn not_worktree(verb: &str, path: &str, repo_dir: &Path) -> Value {
    bad(
        format!(
            "team_git {verb}: `{path}` is not a worktree of `{}`",
            repo_dir.display()
        ),
        "path",
        json!(path),
    )
}

/// Drop the Windows verbatim prefix from a path string: `\\?\UNC\srv\share`
/// becomes `\\srv\share` and `\\?\C:\x` becomes `C:\x`. Anything else
/// (including `\\?\Volume{...}`) is returned unchanged.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn strip_verbatim(s: &str) -> String {
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        let b = rest.as_bytes();
        if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
            return rest.to_string();
        }
    }
    s.to_string()
}

/// On Windows, `canonicalize` yields verbatim (`\\?\`) paths that git
/// mis-parses as arguments. Strip the prefix so every path we hand out or
/// compare is the plain form. Other platforms: unchanged.
#[cfg(windows)]
pub(crate) fn simplify(p: PathBuf) -> PathBuf {
    let Some(s) = p.to_str() else { return p };
    let t = strip_verbatim(s);
    if t == s {
        p
    } else {
        PathBuf::from(t)
    }
}

#[cfg(not(windows))]
pub(crate) fn simplify(p: PathBuf) -> PathBuf {
    p
}

/// Canonicalize `p`, then simplify it. `None` when it does not exist.
pub(crate) fn canon(p: &Path) -> Option<PathBuf> {
    p.canonicalize().ok().map(simplify)
}

/// Resolve `raw` against the root and clamp it. Returns the canonical path,
/// which must be strictly inside the canonical root.
pub(crate) fn inside_root(root: &Path, raw: &str, verb: &str) -> Result<PathBuf, Value> {
    if Path::new(raw)
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(outside(verb, raw, root));
    }
    let abs = if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        root.join(raw)
    };
    let canon_root = canon(root).ok_or_else(|| {
        json!({"error": format!("team_git: workspace_root `{}` does not exist", root.display())})
    })?;
    // Canonicalize the nearest existing ancestor, then re-join the missing tail.
    let mut existing = abs.clone();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        match existing.file_name() {
            Some(name) => tail.push(name.to_os_string()),
            None => break,
        }
        if !existing.pop() {
            break;
        }
    }
    let mut resolved = canon(&existing).unwrap_or(existing);
    for part in tail.iter().rev() {
        resolved.push(part);
    }
    if !resolved.starts_with(&canon_root) || resolved == canon_root {
        return Err(outside(verb, raw, root));
    }
    Ok(resolved)
}

/// True when `path` is a worktree of `cfg.repo_dir`: same git common dir, and
/// its own top level is `path`. Any git failure is false.
pub(crate) async fn is_worktree_of(cfg: &Cfg, path: &Path) -> bool {
    let common_here = git::common_dir(path).await;
    let common_repo = git::common_dir(&cfg.repo_dir).await;
    let top = git::toplevel(path).await;
    match (common_here, common_repo, top) {
        (Some(a), Some(b), Some(t)) => a == b && t == canon(path).unwrap_or(path.to_path_buf()),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_verbatim_drops_the_drive_prefix() {
        assert_eq!(
            strip_verbatim(r"\\?\C:\Users\runneradmin\ws\w1"),
            r"C:\Users\runneradmin\ws\w1"
        );
    }

    #[test]
    fn strip_verbatim_maps_unc_and_leaves_other_paths() {
        assert_eq!(strip_verbatim(r"\\?\UNC\srv\share\x"), r"\\srv\share\x");
        assert_eq!(strip_verbatim("/tmp/a/b"), "/tmp/a/b");
        assert_eq!(strip_verbatim(r"C:\a\b"), r"C:\a\b");
        assert_eq!(strip_verbatim(r"\\?\Volume{1234}\a"), r"\\?\Volume{1234}\a");
    }

    #[test]
    fn inside_root_never_returns_a_verbatim_path() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join("sub")).unwrap();
        let canon_root = simplify(root.canonicalize().unwrap());
        let w = inside_root(root, "w1", "t").unwrap();
        assert!(!w.to_string_lossy().starts_with(r"\\?\"), "{w:?}");
        assert!(w.starts_with(&canon_root), "{w:?}");
        assert_eq!(w, canon_root.join("w1"));
        let n = inside_root(root, "sub/new", "t").unwrap();
        assert!(!n.to_string_lossy().starts_with(r"\\?\"), "{n:?}");
        assert_eq!(n, canon_root.join("sub").join("new"));
    }

    #[test]
    fn a_nonexistent_tail_is_rejoined_per_component() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let canon_root = simplify(root.canonicalize().unwrap());
        let got = inside_root(root, "a/b/c", "t").unwrap();
        let expect = canon_root.join("a").join("b").join("c");
        assert_eq!(
            got.components().collect::<Vec<_>>(),
            expect.components().collect::<Vec<_>>()
        );
        if cfg!(windows) {
            assert!(!got.to_string_lossy().contains('/'), "{got:?}");
        }
    }
}
