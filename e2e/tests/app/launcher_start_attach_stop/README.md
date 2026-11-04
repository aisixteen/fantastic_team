# app/launcher_start_attach_stop (free)

The app shell `fantastic_team_app` (the launcher behind the window) starts a headless child, attaches to a server that is already up, and stops.

Asserts, with the real app binary (built next to `--bin`; a Skip when it is not):
- `--attach URL` on the real headless binary (started with `canvas --port`, serving the canvas page): the app prints `web UI at URL`, starts no child process, exits 0 on SIGINT, and the server it attached to is still up (attach never stops a server). `--attach` to an unused port exits non-zero ("no 200"), with no child.
- `--binary B --port P --dir D`: the app prints `web UI at http://127.0.0.1:P/canvas_page/file/index.html`, has exactly one child process, the child's command line ends with `canvas --port P` (nothing else; it runs in D), it answers 200 at the canvas page and 404 at `/`; SIGINT makes the app exit 0, the child is gone (the pid) and the port refuses connections.

The START leg runs the real binary (`--binary BIN`) as fantastic's `canvas --port P` in the start directory. The launcher waits for a 200 at `/canvas_page/file/index.html` (`READY_PATH`); the web host answers 404 at `/`. The probe first runs the binary once in the start directory to seed it (a never-used directory serves the page from its second `canvas` start). The directory brings its own `blueprint/` (the test's), because the launcher writes the app's preset into a directory without one and imports it.

Found while moving this test (not fixed here, it is the app's): the launcher against a brand-new directory with no `blueprint/` writes the preset, starts `canvas --port P`, and then fails with `the preset import was refused: bind @credentials: no agent "credentials"` and exit 1, because a fresh home has no `credentials` agent (`fantastic ai ...` makes it when a key is saved) and nothing in the app makes one before the import.
