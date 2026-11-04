# app/launcher_start_attach_stop (free)

The app shell `fantastic_team_app` (the launcher behind the window) starts a headless child, attaches to a server that is already up, and stops.

Asserts, with the real app binary (built next to `--bin`; a Skip when it is not):
- `--attach URL` on the real headless binary (started with `canvas --port`, serving the canvas page): the app prints `web UI at URL`, starts no child process, exits 0 on SIGINT, and the server it attached to is still up (attach never stops a server). `--attach` to an unused port exits non-zero ("no 200"), with no child.
- `--binary B --port P --dir D`: the app prints `web UI at http://127.0.0.1:P/canvas_page/file/index.html`, has exactly one child process, the child's command line ends with `canvas --port P` (nothing else; it runs in D), it answers 200 at the canvas page and 404 at `/`; SIGINT makes the app exit 0, the child is gone (the pid) and the port refuses connections.

The START leg runs the real binary (`--binary BIN`) as fantastic's `canvas --port P` in the start directory. The launcher waits for a 200 at `/canvas_page/file/index.html` (`READY_PATH`); the web host answers 404 at `/`. The probe first runs the binary once in the start directory to seed it (a never-used directory serves the page from its second `canvas` start) and makes a `lead` there with `send`: the launcher makes the team only when the kernel has no `lead`, so this directory is started as it is.

Second START leg, a folder with no team: seeded once, given the home's `credentials` (a fresh home has none; `fantastic ai ...` makes it when a key is saved, and the new team's lead, `github` and the tiers name it), then started by the app. The app runs `create_agent` for `lead`, `queue`, `github`, `board`, `roles`, `tasks`, `worker_exec` and `worker_files` under those fixed ids (no import, so no `_1` suffix), writes the role cards under `roles/`, and merges the lead into the `callable_by` of the home's `memory` (which keeps `user`). Without `credentials` the app refuses with "the agent `credentials` the team reaches does not exist".
