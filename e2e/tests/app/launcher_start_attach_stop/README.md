# app/launcher_start_attach_stop (free)

The app shell `fantastic_team_app` (the launcher behind the window) starts a headless child, attaches to a server that is already up, and stops.

Asserts, with the real app binary (built next to `--bin`; a Skip when it is not):
- `--attach URL` on the real headless binary (started with `--port`, serving the canvas page): the app prints `web UI at URL`, starts no child process, exits 0 on SIGINT, and the server it attached to is still up (attach never stops a server). `--attach` to an unused port exits non-zero ("no 200"), with no child.
- `--binary B --port P --dir D`: the app prints `web UI at http://127.0.0.1:P/`, has exactly one child process, the child got `--port P --dir D` and nothing else, answers 200; SIGINT makes the app exit 0, the child is gone (the pid) and the port refuses connections.

The START leg runs a stand-in for the headless binary (`headless`, a shell wrapper around a ten-line python server written by the probe), not the real one. The reason is a product bug this test found: the launcher polls `http://127.0.0.1:P/` for a 200 (`web_url`, `wait_ready`), but the real binary's `--port` serves the canvas at `/canvas_page/file/index.html`, and fantastic's `web` host answers 404 at `/` ("an undeclared `/` is a 404"). So `fantastic_team_app` with the real binary exits after 30 s with "no 200 from http://127.0.0.1:P/ within 30s". Fixing it means choosing the URL the window loads (`app/tauri.conf.json`, `patches/github-exec.patch`, `crates/team/tests/app_packaging.rs`, `window_spec_matches_tauri_conf`), which is the maintainer's call; when it is fixed, the START leg should run the real binary (`--binary BIN`) and the stand-in goes.

Mutation: in the probe, kill the app with SIGKILL instead of SIGINT (so it cannot stop its child); "stop leaves no child process" fails. Or point the attach at a started server and expect a child: "attach starts no child process" holds only because attach spawns nothing.
