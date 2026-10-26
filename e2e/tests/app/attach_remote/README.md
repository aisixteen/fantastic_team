# app/attach_remote (free)

The app attaches to a shift on another box through the user's own `ssh -L`, and nothing secret goes through the app or into a log.

The shift is the real binary on a port (with a fake model key, app id and PEM in ITS environment only). The "remote box" is a front on a second port; `fake_ssh` stands in for the user's ssh: it records its argv and forwards the loopback port it is asked for. The app runs `--tunnel fakehost:REMOTE --ssh fake_ssh`.

Asserts: the app prints a loopback URL (`http://127.0.0.1:LOCAL/`); the page answers through the tunnel; ssh's argv is exactly `-N -o ExitOnForwardFailure=yes -L 127.0.0.1:LOCAL:127.0.0.1:REMOTE fakehost` (bound to loopback only, no key, token, URL or user info); ssh is the app's child; the model key and the app key text appear in no file of the run (outside `secrets/`), in no output of the app and not in the page; SIGINT makes the app exit 0, ssh gone, the tunnel port closed, and the shift still running. `--attach http://user:PASSWORD@...` exits 2, starts nothing, and never echoes the password; `--tunnel -oProxyCommand=...` (a host ssh would read as an option) exits 2 and runs no ssh.

The remote front maps `/` to the canvas page. Reason (a product bug this suite found, see `app/launcher_start_attach_stop`): the launcher waits for a 200 at `/`, and the real web host answers 404 there, so against the real binary alone `--tunnel` (like `--port`) is never ready.

Mutation: make the fake ssh add `-i KEY` to its recorded argv, or write the model key into `ssh_argv.txt`; the argv and secret checks fail.
