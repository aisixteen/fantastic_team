//! `--worktrees DIR`: the folder the git connectors' default worktree roots go in
//! (`<DIR>/<agent id>`), for every git connector whose record names no
//! `workspace_root`. Without it the folder is the system temp folder's
//! `fantastic_team/worktrees`. The team cuts the flag out before fantastic parses
//! the rest, as it does `webui`.

use std::path::PathBuf;

/// Cut `--worktrees DIR` out of `argv`. Returns the folder, if named, and the rest.
pub fn split(argv: &[String]) -> Result<(Option<PathBuf>, Vec<String>), String> {
    let mut dir: Option<PathBuf> = None;
    let mut rest = Vec::with_capacity(argv.len());
    let mut i = 0;
    while i < argv.len() {
        if argv[i] != "--worktrees" {
            rest.push(argv[i].clone());
            i += 1;
            continue;
        }
        let value = argv
            .get(i + 1)
            .filter(|v| !v.is_empty() && !v.starts_with("--"))
            .ok_or("--worktrees needs a folder")?;
        if dir.is_some() {
            return Err("--worktrees is named twice".into());
        }
        let p = PathBuf::from(value);
        if !p.is_absolute() {
            return Err(format!("--worktrees needs an absolute path, not `{value}`"));
        }
        dir = Some(p);
        i += 2;
    }
    Ok((dir, rest))
}

#[cfg(test)]
mod tests {
    use super::split;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_flag_is_cut_out_and_the_rest_kept() {
        let (dir, rest) = split(&v(&[
            "--headless",
            "--worktrees",
            "/w/wt",
            "webui",
            "--port",
            "8",
        ]))
        .unwrap();
        assert_eq!(dir.unwrap().to_str(), Some("/w/wt"));
        assert_eq!(rest, v(&["--headless", "webui", "--port", "8"]));
    }

    #[test]
    fn without_the_flag_nothing_changes() {
        let (dir, rest) = split(&v(&["--headless"])).unwrap();
        assert!(dir.is_none());
        assert_eq!(rest, v(&["--headless"]));
    }

    #[test]
    fn a_relative_missing_or_doubled_folder_is_refused() {
        assert!(split(&v(&["--worktrees", "wt"]))
            .unwrap_err()
            .contains("absolute"));
        assert!(split(&v(&["--worktrees"]))
            .unwrap_err()
            .contains("needs a folder"));
        assert!(split(&v(&["--worktrees", "--headless"]))
            .unwrap_err()
            .contains("needs a folder"));
        assert!(split(&v(&["--worktrees", "/a", "--worktrees", "/b"]))
            .unwrap_err()
            .contains("twice"));
    }
}
