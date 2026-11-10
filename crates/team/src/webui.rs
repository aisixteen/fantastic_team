//! The `webui` word: `fantastic_team webui [--port N] [--host ADDR]` serves the team
//! page from the binary, the way fantastic's `canvas` serves the canvas.
//!
//! The word is cut out by [`split`] and its flags are passed to fantastic's
//! `--headless --port --host`; the browser is the View.
//!
//! `webui` and `canvas` together are refused: both mount a `web_ws` leg on the one
//! `/{host_id}/ws` route (`team_ws` and `canvas_ws`), and the second wins, so the
//! other page would load and never connect.

/// What `webui` was given.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WebUi {
    /// `--port N`; none keeps the recorded port, else picks a free one.
    pub port: Option<u16>,
    /// `--host ADDR`: the address the web host binds (loopback unless named).
    pub bind: Option<String>,
}

/// Why the two are refused together.
pub const WITH_CANVAS: &str = "`webui` and `canvas` cannot run together: both put a socket leg on the web host's one /{host_id}/ws route and the second would win; run one of them";

/// The page the web host serves for `webui`.
pub fn page_url(host: &str, port: u16) -> String {
    let authority = if crate::app_control::is_wildcard(host) {
        format!("localhost:{port}")
    } else if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    format!("http://{authority}{}", crate::page::PAGE_PATH)
}

fn port_of(v: &str) -> Result<u16, String> {
    v.parse::<u16>()
        .ok()
        .filter(|p| *p > 0)
        .ok_or_else(|| format!("--port must be 1..65535, got `{v}`"))
}

fn bind_of(v: &str) -> Result<String, String> {
    v.parse::<std::net::IpAddr>()
        .map(|_| v.to_string())
        .map_err(|_| format!("--host must be an IP address, got `{v}`"))
}

/// Cut `webui [--port N] [--host ADDR]` out of `argv`. Returns what it was given (none
/// when the word is absent) and the argv fantastic parses: the rest, with `--headless`
/// added since the browser is the View (unless it names it, or `canvas`, itself).
/// `--port` and `--host` count as `webui`'s only directly after it.
pub fn split(argv: &[String]) -> Result<(Option<WebUi>, Vec<String>), String> {
    let Some(at) = argv.iter().position(|a| a == "webui") else {
        return Ok((None, argv.to_vec()));
    };
    let mut w = WebUi::default();
    let mut rest: Vec<String> = argv[..at].to_vec();
    let mut i = at + 1;
    while let Some(flag) = argv.get(i).map(String::as_str) {
        let value = |what: &str| {
            argv.get(i + 1)
                .filter(|v| !v.is_empty())
                .ok_or_else(|| what.to_string())
        };
        match flag {
            "--port" => w.port = Some(port_of(value("--port needs a number")?)?),
            "--host" => w.bind = Some(bind_of(value("--host needs an IP address")?)?),
            _ => break,
        }
        i += 2;
    }
    rest.extend_from_slice(&argv[i..]);
    if rest.iter().any(|a| a == "webui") {
        return Err("`webui` is named twice".into());
    }
    if rest.iter().any(|a| a == "canvas") {
        return Err(WITH_CANVAS.into());
    }
    if rest.iter().any(|a| a == "run") {
        return Err("`run` needs a View: the terminal UI or the canvas; `webui` is neither".into());
    }
    if !rest.iter().any(|a| a == "--headless") {
        rest.insert(0, "--headless".into());
    }
    Ok((Some(w), rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn no_word_leaves_the_argv_to_fantastic() {
        for line in [
            "",
            "--headless",
            "canvas --port 8123",
            "ai anthropic --model m",
        ] {
            assert_eq!(split(&s(line)).unwrap(), (None, s(line)));
        }
    }

    #[test]
    fn the_word_takes_port_and_host_in_any_order() {
        let (w, rest) = split(&s("webui --port 8123 --host 0.0.0.0")).unwrap();
        assert_eq!(
            w,
            Some(WebUi {
                port: Some(8123),
                bind: Some("0.0.0.0".into())
            })
        );
        assert_eq!(rest, s("--headless"));
        let (w, _) = split(&s("webui --host ::1 --port 9")).unwrap();
        assert_eq!(
            w,
            Some(WebUi {
                port: Some(9),
                bind: Some("::1".into())
            })
        );
        let (w, rest) = split(&s("webui")).unwrap();
        assert_eq!(w, Some(WebUi::default()));
        assert_eq!(rest, s("--headless"));
    }

    #[test]
    fn the_other_words_go_to_fantastic_headless() {
        let (w, rest) = split(&s("ai anthropic --model m webui --port 5")).unwrap();
        assert_eq!(w.unwrap().port, Some(5));
        assert_eq!(rest, s("--headless ai anthropic --model m"));
        let (_, rest) = split(&s("webui --headless")).unwrap();
        assert_eq!(rest, s("--headless"), "--headless is not doubled");
        // a flag after another word is that word's, left for fantastic to judge
        let (w, rest) = split(&s("webui system --port 5")).unwrap();
        assert_eq!(w, Some(WebUi::default()));
        assert_eq!(rest, s("--headless system --port 5"));
    }

    #[test]
    fn bad_flags_are_refused_in_fantastic_s_words() {
        for (line, want) in [
            ("webui --port", "--port needs a number"),
            ("webui --port 0", "--port must be 1..65535, got `0`"),
            ("webui --port 70000", "--port must be 1..65535, got `70000`"),
            ("webui --port x", "--port must be 1..65535, got `x`"),
            ("webui --host", "--host needs an IP address"),
            (
                "webui --host localhost",
                "--host must be an IP address, got `localhost`",
            ),
            ("webui webui", "named twice"),
        ] {
            let e = split(&s(line)).unwrap_err();
            assert!(e.contains(want), "{line}: {e}");
        }
    }

    #[test]
    fn canvas_and_run_do_not_go_with_it() {
        for line in [
            "webui canvas",
            "canvas webui --port 5",
            "webui --port 5 ai canvas",
        ] {
            assert_eq!(split(&s(line)).unwrap_err(), WITH_CANVAS, "{line}");
        }
        let e = split(&s("webui run htop")).unwrap_err();
        assert!(e.contains("`run` needs a View"), "{e}");
    }

    #[test]
    fn the_page_url_names_a_dialable_host() {
        assert_eq!(
            page_url("127.0.0.1", 8123),
            "http://127.0.0.1:8123/team_page/file/index.html"
        );
        assert_eq!(
            page_url("0.0.0.0", 8123),
            "http://localhost:8123/team_page/file/index.html"
        );
        assert_eq!(
            page_url("::1", 8123),
            "http://[::1]:8123/team_page/file/index.html"
        );
    }
}
