"""cli_ssh/headless_and_canvas: the local kernel serves `canvas --port P` (then `webui --port P2`),
the remote kernel in a Podman box runs `--headless`. The lead wires the remote through the bridge
and makes `rlead` there; `rlead` makes a canvas (`create_agent` of a `canvas_page` under itself, no
`canvas` word) and both add a note to the shared NOTES.md (the lead's commit here, rlead's in the box).
The local canvas's socket and then the team page's socket list the remote agents through the bridge;
the box listens on sshd and the tunnel's port only. Then everything made on the remote is deleted
through the bridge, the bridge deleted, the remote stopped. Then the local test's last checks:
`webui` with `canvas` is refused, `--headless` opens no port. The wiring and the shared checks are
`helpers/ssh_twins.py`'s."""

import os
import subprocess
import time

from helpers import cli, cli_scenarios as S, podman_remote, ssh_twins as T, team as team_mod
from helpers.verdict import Skip, Verdict

def fresh(ctx) -> None:
    sock = os.path.join(ctx.work, team_mod.SOCK)
    if os.path.exists(sock):  # the stopped team's socket file must not pass for the next start
        os.remove(sock)


def run(ctx):
    v = Verdict("cli_ssh/headless_and_canvas")
    podman_remote.require()
    if cli.listeners(os.getpid()) is None:
        raise Skip("neither lsof nor /proc/net/tcp is available to list listening sockets")
    one, pause, two, three = T.CANVAS_PHASES
    with T.Box(ctx) as box:
        port = team_mod.free_port()
        team = cli.start(ctx, extra_args=["canvas", "--port", port])
        cli.add_base(ctx)
        base = set(cli.ids(team))
        cli.make_lead(ctx, {cli.EXEC: "*", "kernel_state": ["create_agent", "delete_agent"]})
        v.check("the lead was made with create_agent", set(cli.ids(team)) - base == {"lead"})
        before = cli.tree(team)
        box.publish()
        box.bind()
        v.check("canvas --port P serves the canvas page", S.wait_page(port, "/canvas_page/file/index.html") == 200)

        lanes = len(ctx.model.turns) - sum(T.CANVAS_PHASES)
        v.check("1: the lead wired the remote, rlead made its canvas and its note",
                T.hand(ctx, cli.LEAD, "Wire the remote kernel and have rlead there make its canvas and note.",
                       upto=one + lanes), f"served {ctx.model.served}")
        v.check("rlead asked the model from the remote kernel", "r" in ctx.model.request_lanes)
        v.check("the box listens on sshd and the tunnel's port only (the remote is --headless)",
                box.listening() <= {22, box.tunnel_port}, str(box.listening()))
        ports = cli.listeners(team.proc.pid)
        v.check("the local kernel listens on the canvas port and the tunnel's local end only (the bridge dials out)",
                ports == {str(port), str(box.tunnel_port)}, str(ports))
        T.shows_canvas(v, "the local canvas's socket (canvas_ws -> user.forward -> the bridge)",
                       T.ask_through(port, "/canvas_ws/ws"))
        v.check("1b: the lead deleted the bridge and stopped the remote kernel", T.hand(ctx, cli.LEAD, "Delete the bridge and stop the remote kernel for now.",
                                                              upto=one + lanes + pause), f"served {ctx.model.served}")
        team.stop()

        wport = team_mod.free_port()
        fresh(ctx)
        team.start(extra_args=["webui", "--port", wport])
        v.check("webui --port P2 serves the team page", S.wait_page(wport, "/team_page/file/index.html") == 200)
        v.check("2: the lead started the remote again and made the bridge again",
                T.hand(ctx, cli.LEAD, "Start the remote kernel again and make the bridge again.",
                       upto=one + lanes + pause + two), f"served {ctx.model.served}")
        T.shows_canvas(v, "the team page's socket (/team_ws/ws -> user.forward -> the bridge)", T.ask_through(wport))
        v.check("3: the lead deleted the remote agents, the bridge, and stopped the remote",
                T.hand(ctx, cli.LEAD, "Clean up the remote and stop it.") and ctx.model.consumed,
                f"served {ctx.model.served} of {len(ctx.model.turns)}")
        after = cli.tree(team)
        extra, missing = set(after) - set(before), set(before) - set(after)
        v.check("the local records are the ones it started with, plus the team page's (the bridge deleted)",
                extra <= {"team_page", "team_ws"} and not missing, f"extra {sorted(extra)} missing {sorted(missing)}")
        box.end(v, solved=False)
        notes = box.server_files.get("NOTES.md", "")
        v.check("both notes landed on the git server: the lead's and rlead's",
                "local: the notes of the lead" in notes and "remote: the notes of rlead" in notes, notes)
        team.stop()

        fresh(ctx)
        both = subprocess.run(team.argv(["webui", "canvas", "--port", team_mod.free_port()]), env=team.env,
                              cwd=team.dir, capture_output=True, text=True, timeout=60, stdin=subprocess.DEVNULL)
        v.check("webui with canvas is refused (exit 2) and says why",
                both.returncode == 2 and "`webui` and `canvas` cannot run together" in both.stderr,
                f"{both.returncode} {both.stderr[-300:]}")
        team.start()
        time.sleep(2)
        ports = cli.listeners(team.proc.pid)
        v.check("--headless opens no port here either", ports == set(), str(ports))
        team.stop()
    box.removed(v)
    return v.finish()
