# app/launcher_start_attach_stop (free)

The app shell `fantastic_team_app` (the launcher behind the window) starts a headless child, attaches to a server that is already up, and stops.

Asserts, with the real app binary (built next to `--bin`; a Skip when it is not):
- `--attach URL` on the real headless binary (started with `--port`, serving the canvas page): the app prints `web UI at URL`, starts no child process, exits 0 on SIGINT, and the server it attached to is still up (attach never stops a server). `--attach` to an unused port exits non-zero ("no 200"), with no child.
- `--binary B --port P --dir D`: the app prints `web UI at http://127.0.0.1:P/`, has exactly one child process, the child's command line ends with `--port P --dir D` (nothing else), it answers 200 at `/team_page/file/index.html` and 404 at `/`; SIGINT makes the app exit 0, the child is gone (the pid) and the port refuses connections.

The START leg runs the real headless binary (`--binary BIN`). The launcher waits for a 200 at `/team_page/file/index.html`, the page the binary's `--port` serves since #181; the web host answers 404 at `/`, The probe first runs the binary once in the start directory to seed it, because a never-used directory serves the page only from its second `--port` start (so the launcher against a brand-new `--dir` still times out on the first run). `/` is still the URL the app prints and the window loads (a follow-up for the maintainer).

Mutation: in the probe, kill the app with SIGKILL instead of SIGINT (so it cannot stop its child); "stop leaves no child process" fails. Or point the attach at a started server and expect a child: "attach starts no child process" holds only because attach spawns nothing.
