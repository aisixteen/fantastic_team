//! `fantastic_team packs <verb> [--key value ...]`: the pack verbs from a shell.
//! Each verb is a message to the team's `pack` or `pack_listing` agent, sent
//! through fantastic's own `send` command, so it needs a kernel running in the
//! directory (the same as `fantastic_team send`). The CLI adds no verb and no
//! record field: it maps the flags onto the payloads the bundles already read.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::tree::{PACK_ID, PACK_LISTING_ID};
use crate::USAGE;

/// One verb: the agent it goes to, its payload type, and its flags. A flag is
/// `(name, kind)`; a required flag is listed in `required`.
struct Verb {
    agent: &'static str,
    kind: &'static str,
    flags: &'static [(&'static str, Kind)],
    required: &'static [&'static str],
}

/// How a flag's value is read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Kept as the string the shell gave.
    Text,
    /// `true` or `false`.
    Bool,
    /// A whole number of seconds.
    Seconds,
    /// A switch: `--name` alone means true, and it takes no value.
    Switch,
}

const VERBS: &[(&str, Verb)] = &[
    (
        "list",
        Verb {
            agent: PACK_LISTING_ID,
            kind: "list",
            flags: &[("query", Kind::Text)],
            required: &[],
        },
    ),
    (
        "show",
        Verb {
            agent: PACK_LISTING_ID,
            kind: "show",
            flags: &[("name", Kind::Text), ("version", Kind::Text)],
            required: &["name"],
        },
    ),
    (
        "updates",
        Verb {
            agent: PACK_LISTING_ID,
            kind: "updates",
            flags: &[("opt_in", Kind::Bool)],
            required: &[],
        },
    ),
    (
        "publish",
        Verb {
            agent: PACK_LISTING_ID,
            kind: "publish",
            flags: &[
                ("pack", Kind::Text),
                ("url", Kind::Text),
                ("signing_key", Kind::Text),
            ],
            required: &["pack", "url", "signing_key"],
        },
    ),
    (
        "fetch",
        Verb {
            agent: PACK_LISTING_ID,
            kind: "fetch",
            flags: &[
                ("name", Kind::Text),
                ("hash", Kind::Text),
                ("stage", Kind::Text),
                ("version", Kind::Text),
            ],
            required: &["name", "hash", "stage"],
        },
    ),
    (
        "submit",
        Verb {
            agent: PACK_LISTING_ID,
            kind: "submit",
            flags: &[
                ("pack", Kind::Text),
                ("url", Kind::Text),
                ("signing_key", Kind::Text),
            ],
            required: &["pack", "url", "signing_key"],
        },
    ),
    (
        "report",
        Verb {
            agent: PACK_LISTING_ID,
            kind: "report",
            flags: &[
                ("name", Kind::Text),
                ("version", Kind::Text),
                ("reason", Kind::Text),
            ],
            required: &["name", "version", "reason"],
        },
    ),
    (
        "takedown",
        Verb {
            agent: PACK_LISTING_ID,
            kind: "takedown",
            flags: &[
                ("name", Kind::Text),
                ("version", Kind::Text),
                ("signing_key", Kind::Text),
            ],
            required: &["name", "version", "signing_key"],
        },
    ),
    (
        "page",
        Verb {
            agent: PACK_LISTING_ID,
            kind: "page",
            flags: &[],
            required: &[],
        },
    ),
    (
        "pack",
        Verb {
            agent: PACK_ID,
            kind: "pack",
            flags: &[("worker", Kind::Text), ("out", Kind::Text)],
            required: &["worker", "out"],
        },
    ),
    (
        "install",
        Verb {
            agent: PACK_ID,
            kind: "install",
            flags: &[
                ("pack", Kind::Text),
                ("into", Kind::Text),
                ("worker", Kind::Text),
                ("timeout_s", Kind::Seconds),
                ("parent_id", Kind::Text),
                ("approve", Kind::Switch),
                ("allow_code", Kind::Switch),
            ],
            required: &["pack", "into"],
        },
    ),
    (
        "eval",
        Verb {
            agent: PACK_ID,
            kind: "eval",
            flags: &[
                ("pack", Kind::Text),
                ("worker", Kind::Text),
                ("timeout_s", Kind::Seconds),
            ],
            required: &["pack", "worker"],
        },
    ),
];

/// Turn `packs <verb> [--key value ...]` into the agent to send to and the payload.
/// A flag name may be spelt with dashes (`--timeout-s`) or underscores (`--timeout_s`).
/// Refuses a missing verb, an unknown verb or flag, a flag without a value, a
/// missing required flag, and a value of the wrong kind; every refusal carries the usage.
pub fn parse(args: &[String]) -> Result<(String, Value), String> {
    let refuse = |why: String| format!("packs: {why}\n{USAGE}");
    let Some((verb_word, rest)) = args.split_first() else {
        return Err(refuse(
            "a verb is needed: list, show, updates, publish, fetch, submit, report, takedown, page, pack, install or eval".into(),
        ));
    };
    let Some((_, verb)) = VERBS.iter().find(|(name, _)| *name == verb_word) else {
        return Err(refuse(format!("unknown verb `{verb_word}`")));
    };
    let mut given: BTreeMap<String, String> = BTreeMap::new();
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        let Some(name) = arg.strip_prefix("--") else {
            return Err(refuse(format!("`{arg}` is not a flag")));
        };
        let key = name.replace('-', "_");
        if !verb.flags.iter().any(|(k, _)| *k == key) {
            return Err(refuse(format!("unknown flag `--{name}` for `{verb_word}`")));
        }
        let is_switch = verb
            .flags
            .iter()
            .any(|(k, kind)| *k == key && *kind == Kind::Switch);
        if is_switch {
            given.insert(key, "true".to_string());
            continue;
        }
        let Some(value) = it.next() else {
            return Err(refuse(format!("`--{name}` needs a value")));
        };
        given.insert(key, value.clone());
    }
    for key in verb.required {
        if !given.contains_key(*key) {
            return Err(refuse(format!(
                "`{verb_word}` needs `--{}`",
                key.replace('_', "-")
            )));
        }
    }
    let mut payload = json!({"type": verb.kind});
    for (key, raw) in &given {
        let kind = verb
            .flags
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, kind)| *kind)
            .unwrap_or(Kind::Text);
        let value = match kind {
            Kind::Text => json!(raw),
            Kind::Switch => json!(true),
            Kind::Bool => match raw.as_str() {
                "true" => json!(true),
                "false" => json!(false),
                _ => {
                    return Err(refuse(format!(
                        "`--{}` must be true or false, got `{raw}`",
                        key.replace('_', "-")
                    )))
                }
            },
            Kind::Seconds => match raw.parse::<u64>() {
                Ok(n) => json!(n),
                Err(_) => {
                    return Err(refuse(format!(
                        "`--timeout-s` must be a whole number of seconds, got `{raw}`"
                    )))
                }
            },
        };
        payload[key.as_str()] = value;
    }
    Ok((verb.agent.to_string(), payload))
}

/// The verb-level error in the row `send` prints (`{pid, status, result}`), if
/// the reply carries one: `result.error`, or a top-level `error`.
pub fn reply_error(stdout: &str) -> Option<String> {
    let row: Value = serde_json::from_str(stdout.trim()).ok()?;
    let err = row
        .get("result")
        .and_then(|r| r.get("error"))
        .or_else(|| row.get("error"))?;
    Some(match err.as_str() {
        Some(s) => s.to_string(),
        None => err.to_string(),
    })
}

/// Run `packs` from the command line: the verb goes to its agent through
/// fantastic's `send`, which needs the team's kernel running in this directory.
/// `send` exits the process itself and exits 0 whenever the item completed, so
/// it runs as a child of this binary; its row is printed as it came, and a
/// verb-level `error` in the reply (or a failed `send`) is returned as Err, which
/// the caller turns into a non-zero exit.
pub fn run_cli(args: &[String]) -> Result<(), String> {
    use std::io::Write as _;
    let (agent, payload) = parse(args)?;
    let item = json!({"target": agent, "payload": payload}).to_string();
    let exe = std::env::current_exe().map_err(|e| format!("packs: no executable path: {e}"))?;
    let out = std::process::Command::new(exe)
        .arg("send")
        .arg(item)
        .stderr(std::process::Stdio::inherit())
        .output()
        .map_err(|e| format!("packs: could not run send: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let _ = write!(std::io::stdout(), "{stdout}");
    if let Some(e) = reply_error(&stdout) {
        return Err(format!("packs: {e}"));
    }
    if !out.status.success() {
        return Err(format!("packs: send failed ({})", out.status));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_maps_each_verb_to_agent_and_payload() {
        assert_eq!(
            parse(&v(&["list"])).unwrap(),
            (PACK_LISTING_ID.to_string(), json!({"type": "list"}))
        );
        assert_eq!(
            parse(&v(&["list", "--query", "git"])).unwrap(),
            (
                PACK_LISTING_ID.to_string(),
                json!({"type": "list", "query": "git"})
            )
        );
        assert_eq!(
            parse(&v(&["show", "--name", "w"])).unwrap(),
            (
                PACK_LISTING_ID.to_string(),
                json!({"type": "show", "name": "w"})
            )
        );
        assert_eq!(
            parse(&v(&["show", "--name", "w", "--version", "1.0"])).unwrap(),
            (
                PACK_LISTING_ID.to_string(),
                json!({"type": "show", "name": "w", "version": "1.0"})
            )
        );
        assert_eq!(
            parse(&v(&["updates"])).unwrap(),
            (PACK_LISTING_ID.to_string(), json!({"type": "updates"}))
        );
        assert_eq!(
            parse(&v(&["updates", "--opt-in", "true"])).unwrap(),
            (
                PACK_LISTING_ID.to_string(),
                json!({"type": "updates", "opt_in": true})
            )
        );
        assert_eq!(
            parse(&v(&["pack", "--worker", "w", "--out", "pack"])).unwrap(),
            (
                PACK_ID.to_string(),
                json!({"type": "pack", "worker": "w", "out": "pack"})
            )
        );
        assert_eq!(
            parse(&v(&["install", "--pack", "pack", "--into", "templates"])).unwrap(),
            (
                PACK_ID.to_string(),
                json!({"type": "install", "pack": "pack", "into": "templates"})
            )
        );
        assert_eq!(
            parse(&v(&[
                "install",
                "--pack",
                "pack",
                "--into",
                "templates",
                "--worker",
                "w",
                "--timeout-s",
                "30"
            ]))
            .unwrap(),
            (
                PACK_ID.to_string(),
                json!({"type": "install", "pack": "pack", "into": "templates", "worker": "w", "timeout_s": 30})
            )
        );
        assert_eq!(
            parse(&v(&["eval", "--pack", "pack", "--worker", "w"])).unwrap(),
            (
                PACK_ID.to_string(),
                json!({"type": "eval", "pack": "pack", "worker": "w"})
            )
        );
        assert_eq!(
            parse(&v(&[
                "eval",
                "--pack",
                "pack",
                "--worker",
                "w",
                "--timeout-s",
                "5"
            ]))
            .unwrap(),
            (
                PACK_ID.to_string(),
                json!({"type": "eval", "pack": "pack", "worker": "w", "timeout_s": 5})
            )
        );
    }

    #[test]
    fn parse_publish_maps_flags() {
        assert_eq!(
            parse(&v(&[
                "publish",
                "--pack",
                "p",
                "--url",
                "https://x",
                "--signing-key",
                "k"
            ]))
            .unwrap(),
            (
                PACK_LISTING_ID.to_string(),
                json!({"type": "publish", "pack": "p", "url": "https://x", "signing_key": "k"})
            )
        );
    }

    #[test]
    fn parse_publish_requires_its_flags() {
        let all = ["--pack", "p", "--url", "https://x", "--signing-key", "k"];
        for skip in 0..3 {
            let mut args = vec!["publish"];
            for (i, pair) in all.chunks(2).enumerate() {
                if i != skip {
                    args.extend(pair);
                }
            }
            let e = parse(&v(&args)).unwrap_err();
            assert!(e.contains("needs") && e.contains("usage:"), "{e}");
        }
        let e = parse(&v(&[])).unwrap_err();
        assert!(e.contains("publish"), "{e}");
    }

    #[test]
    fn reply_error_finds_a_verb_level_error() {
        let bad = r#"{"pid": 1, "status": "completed", "result": {"error": "no such pack"}}"#;
        assert_eq!(reply_error(bad).as_deref(), Some("no such pack"));
        let top = r#"{"status": "failed", "error": {"code": 3}}"#;
        assert!(reply_error(top).is_some());
        let ok = r#"{"pid": 1, "status": "completed", "result": {"packs": []}}"#;
        assert_eq!(reply_error(ok), None);
        assert_eq!(reply_error("not json"), None);
    }

    #[test]
    fn install_flags_parent_id_approve_allow_code() {
        let base = [
            "install",
            "--pack",
            "p",
            "--into",
            "templates",
            "--parent-id",
            "lead",
        ];
        let (agent, payload) = parse(&v(&[&base[..], &["--approve"]].concat())).unwrap();
        assert_eq!(agent, PACK_ID);
        assert_eq!(payload["parent_id"], "lead");
        assert_eq!(payload["approve"], json!(true));
        assert!(payload.get("allow_code").is_none());
        let (_, payload) = parse(&v(&base)).unwrap();
        assert!(payload.get("approve").is_none(), "{payload}");
        let (_, payload) =
            parse(&v(&[&base[..], &["--allow-code", "--approve"]].concat())).unwrap();
        assert_eq!(payload["allow_code"], json!(true));
        assert_eq!(payload["approve"], json!(true));
    }

    #[test]
    fn parse_refuses_missing_required_and_unknown() {
        let bad: [&[&str]; 10] = [
            &["install", "--pack", "pack"],
            &["eval", "--pack", "pack"],
            &["pack", "--worker", "w"],
            &["show"],
            &["frobnicate"],
            &[],
            &["list", "--nope", "x"],
            &[
                "eval",
                "--pack",
                "p",
                "--worker",
                "w",
                "--timeout-s",
                "soon",
            ],
            &["list", "--query"],
            &["updates", "--opt-in", "maybe"],
        ];
        for case in bad {
            let e = parse(&v(case))
                .err()
                .unwrap_or_else(|| panic!("{case:?} must be refused"));
            assert!(e.contains("usage:"), "{case:?}: {e}");
        }
    }

    #[test]
    fn parse_fetch_submit_report_takedown_map_flags() {
        assert_eq!(
            parse(&v(&["fetch", "--name", "n", "--hash", "h", "--stage", "s"])).unwrap(),
            (
                PACK_LISTING_ID.to_string(),
                json!({"type": "fetch", "name": "n", "hash": "h", "stage": "s"})
            )
        );
        assert_eq!(
            parse(&v(&[
                "submit",
                "--pack",
                "p",
                "--url",
                "https://x/y",
                "--signing-key",
                "k"
            ]))
            .unwrap(),
            (
                PACK_LISTING_ID.to_string(),
                json!({"type": "submit", "pack": "p", "url": "https://x/y", "signing_key": "k"})
            )
        );
        assert_eq!(
            parse(&v(&[
                "report",
                "--name",
                "n",
                "--version",
                "1.0",
                "--reason",
                "bad"
            ]))
            .unwrap(),
            (
                PACK_LISTING_ID.to_string(),
                json!({"type": "report", "name": "n", "version": "1.0", "reason": "bad"})
            )
        );
        assert_eq!(
            parse(&v(&[
                "takedown",
                "--name",
                "n",
                "--version",
                "1.0",
                "--signing-key",
                "k"
            ]))
            .unwrap(),
            (
                PACK_LISTING_ID.to_string(),
                json!({"type": "takedown", "name": "n", "version": "1.0", "signing_key": "k"})
            )
        );
        assert_eq!(
            parse(&v(&["page"])).unwrap(),
            (PACK_LISTING_ID.to_string(), json!({"type": "page"}))
        );
    }

    #[test]
    fn parse_new_verbs_require_their_flags() {
        for args in [
            v(&["fetch", "--name", "n", "--hash", "h"]),
            v(&["submit", "--pack", "p", "--url", "u"]),
            v(&["report", "--name", "n", "--version", "1"]),
            v(&["takedown", "--name", "n", "--version", "1"]),
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }
}
