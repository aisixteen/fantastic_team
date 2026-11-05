"""gpu_cli_ssh/headless_and_canvas: `cli_ssh/headless_and_canvas` on a real model, the canvas half: the
local kernel serves `canvas --port P`, the remote runs `--headless`; told the box and the wiring in plain
words, the lead makes `rlead` on the remote, which makes its canvas; both add a note. The local canvas's
socket lists the remote agents through the bridge; the box listens on sshd and the tunnel's port only.
Then the lead cleans up (a second message). The webui half is the scripted twin's. The box and the
checks are `helpers/ssh_twins.py`'s; the task is `helpers/gpu_ssh.py`'s; no script."""

from helpers import cli, cli_scenarios as S, gpu_cli as gpu, gpu_ssh as G, ssh_twins as T, team as team_mod, web
from helpers.verdict import Verdict

ASK = {"type": "forward", "target": "user", "payload": {"type": "forward", "target": "kernel",
                                                         "payload": {"type": "list_agents"}}}


def run(ctx):
    v = Verdict("gpu_cli_ssh/headless_and_canvas")
    G.require(ctx)
    with T.Box(ctx) as box:
        port = team_mod.free_port()
        team = cli.start(ctx, extra_args=["canvas", "--port", port])
        cli.add_base(ctx)
        cli.make_lead(ctx, {cli.EXEC: "*", "kernel_state": ["create_agent", "delete_agent"]})
        before = cli.tree(team)
        box.publish()
        v.check("canvas --port P serves the canvas page", S.wait_page(port, "/canvas_page/file/index.html") == 200)
        watch = G.play(ctx, v, "lead", G.wiring(ctx, box) + G.TASKS["headless_and_canvas"], label="1: ")
        v.check("the box listens on sshd and the tunnel's port only", box.listening() <= {22, box.tunnel_port},
                str(box.listening()))
        try:
            ws = web.WsClient(int(port), path="/canvas_ws/ws")
            reply = ws.call("user", {"type": "forward", "target": T.BRIDGE, "payload": ASK}, timeout=60)
            ws.close()
        except OSError as e:
            reply = {"error": str(e)}
        agents = {a.get("id"): a for a in ((reply or {}).get("agents") or [])}
        sk = agents.get("sketch") or {}
        v.check("the local canvas's socket lists rlead and its canvas through the bridge",
                "rlead" in agents and sk.get("handler_module") == "canvas_page.tools", str(reply)[:400])
        G.play(ctx, v, "lead", G.CLEANUP, label="2: ")
        after = cli.tree(team)
        v.check("the local records are the ones it started with", after == before,
                f"before {sorted(before)} after {sorted(after)}")
        box.end(v, solved=False)
        notes = box.server_files.get("NOTES.md", "")
        v.check("both notes landed on the git server", "notes of the lead" in notes and "notes of rlead" in notes, notes)
        gpu.report(ctx, watch)
    box.removed(v)
    return v.finish()
