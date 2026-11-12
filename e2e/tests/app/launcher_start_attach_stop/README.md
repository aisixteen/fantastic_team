# app/launcher_start_attach_stop (free)

The app shell `fantastic_team_app` (the launcher behind the window) starts a headless child, attaches to a server that is already up, and stops.

Asserts, with the real app binary (built next to `--bin`; a Skip when it is not):
- `--attach URL` on the real headless binary (started with `webui --port`, serving the team page): the app prints `web UI at URL`, starts no child process, exits 0 on SIGINT, and the server it attached to is still up (attach never stops a server). `--attach` to an unused port exits non-zero ("no 200"), with no child.
- `--binary B --port P --dir D`: the app prints `web UI at http://127.0.0.1:P/team_page/file/index.html`, has exactly one child process, the child's command line ends with `webui --port P` (nothing else; it runs in D), it answers 200 at the team page and 404 at `/`; SIGINT makes the app exit 0, the child is gone (the pid) and the port refuses connections.

The START leg runs the real binary (`--binary BIN`) as the team's `webui --port P` in the start directory. The launcher waits for a 200 at `/team_page/file/index.html` (`READY_PATH`); the web host answers 404 at `/`. The probe first runs the binary once in the start directory to seed it (a never-used directory is seeded by the first start) and makes a `lead` there with `send`: the launcher makes the team only when the kernel has no `lead`, so this directory is started as it is.

Second START leg, an empty folder on a fresh home (#222, #261): seeded once by the first `webui` run from `crates/team/blueprint.yaml` (ids as written, no `_1` suffix), then started by the app. The app creates no agent and chooses no key mode: the key mode is the person's, so the probe asserts it is not chosen, that no `credentials` store and no helper agent exist, that the team is there under its fixed ids, that the yaml's parent entries wrote the other end of the edges on the stock agents (`memory` for `bundles` next to the lead's and the stock grants, `user` for the page leg), that the bundles registry serves the repo's bundles (compiled in, no card written to the folder) and that the app exits 0 on SIGINT.
