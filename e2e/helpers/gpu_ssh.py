"""The gpu tier's ssh twins (#242): the six cli scenarios split across the local kernel and one in a
Podman box, on a real model (the vllm plug, no script).

The box, the fixture on the test's git server and the checks are `helpers/ssh_twins.py`'s, the
same as the scripted twins'. Here the lead is told in plain words where the box is (host, port,
key path, the login, the tunnel's port, the git server's URL as the box dials it) and what the weak
wiring is; it writes every command itself. Nothing asserts the model's own path.

Two limits of this tier: the vllm plug counts the usage of the LOCAL kernel's agents (it polls the
team it watches), so the remote agents' requests are not in `cost.json` nor under its cap; and a
keyed server (`VLLM_API_KEY`) is a SKIP: the box holds no key and no credential store.
"""

from __future__ import annotations

import json
import time

from . import cli, gpu_cli as gpu, podman_remote as pr, ssh_twins as T
from .verdict import Skip


def require(ctx) -> None:
    pr.require()
    if ctx.model.keyed:
        raise Skip("the gpu ssh twins need a keyless endpoint: the box holds no key (VLLM_API_KEY is set)")


def remote_record(ctx, id: str, parent: str = "user") -> dict:
    """The fields of an AI agent on the remote kernel: the plug's record, the endpoint as the box dials it."""
    rec = dict(ctx.model.ai_record(), id=id, parent_id=parent, memory_store_id="memory")
    rec["endpoint"] = pr.gateway_url(rec["endpoint"])
    return rec


def wiring(ctx, box: T.Box, roles: dict[str, str] | None = None) -> str:
    """The remote and the weak wiring, in plain words, with this run's values."""
    vals = box.values()
    ssh = " ".join(pr.ssh_argv(vals["ssh_port"], vals["ssh_key"]))
    start = T.kernel_start_command(roles is not None)
    cards = ""
    if roles:
        cards = (" Write these role cards into `~/w/roles/` in the box (one file each, the text exactly): "
                 + json.dumps(roles) + ".")
    return (
        "THE REMOTE. A Linux box you reach with ssh only: `" + ssh + "` (login `team`, host 127.0.0.1, port "
        f"{vals['ssh_port']}, key {vals['ssh_key']}). It has `fantastic_team`, git and python3. Your exec agent "
        f"`{cli.EXEC}` runs the commands (verbs `run` and `start`; `start` keeps a job running). "
        "THE WIRING, step by step: "
        f"(1) With `start`, run ssh with `-o ExitOnForwardFailure=yes -L {vals['tunnel_port']}:127.0.0.1:"
        f"{vals['tunnel_port']}` and the remote command `{start}`: the tunnel and the remote kernel, in one job. "
        "(2) In the box (`ssh ... '<command>'`, in `~/w` once `~/w/.fantastic/cli/mcp.sock` exists), use "
        "`fantastic_team send '<json item>'` to make, on the remote kernel: `{\"target\":\"kernel\",\"payload\":"
        f"{{\"type\":\"create_agent\",\"id\":\"{T.WEB}\",\"parent_id\":\"user\",\"handler_module\":\"web.tools\","
        f"\"port\":{vals['tunnel_port']}}}}}`; the socket leg `{T.LEG}` under it (`web_ws.tools`, "
        f"`ingress_rule: allow_all_ingress`, `callable_by` `{{\"$self\":\"*\",\"{T.WEB}\":[\"get_routes\"]}}`, "
        f"`can_call` `{{\"$self\":\"*\",\"user\":[\"forward\"]}}`); `update_agent` of `{T.WEB}` with `can_call` "
        f"`{{\"$self\":\"*\",\"{T.LEG}\":[\"get_routes\"]}}`; on `user`'s `callable_by` (read it with "
        f"`list_agents`, keep every entry, `update_agent` replaces the whole map) add `\"{T.LEG}\": [\"forward\", "
        f"\"send\"]`; the exec agent `{cli.EXEC}` there: `{json.dumps({k: v for k, v in T.REMOTE_EXEC.items() if k != 'id'})}`; "
        f"then send `{{\"type\":\"boot\"}}` to `{T.WEB}`." + cards + " "
        f"(3) In the box: `cd ~/w && git clone {vals['remote_git_url']} {T.REPO}` (the box's git config "
        "carries the git server's credential). "
        f"(4) On your kernel, `create_agent` the bridge: `{json.dumps(dict(T.bridge_record(), port=int(vals['tunnel_port'])))}`. "
        f"(5) A call on the remote kernel is `forward` on `{T.BRIDGE}` with `target` `user` and `payload` "
        "`{\"type\":\"forward\",\"target\":\"<remote agent id>\",\"payload\":{<the call>}}`; its reply is the "
        f"remote's answer. An AI agent on the remote kernel is a `create_agent` there with these fields: "
        f"{json.dumps({k: v for k, v in remote_record(ctx, '<id>').items()})}. "
        "(6) At the end: delete every agent you made on the remote kernel (through the bridge), delete the "
        f"bridge, stop the remote kernel with `ssh ... '{T.stop_command()}'` (the binary has no stop word; "
        "the tunnel job ends with it), and pull your local clone (`git pull --ff-only`). "
        "Both clones push to the same git server: `git push origin HEAD:main`; pull before you change a file "
        "the other side pushed. ")


TASKS = {
    "single_lead": (
        gpu.BROKEN + "Fix adder.py in your local clone, commit and push; then, in your workspace on the remote "
        f"box (the remote kernel's `{cli.EXEC}`, through the bridge), pull, fix muler.py, commit and push. " + gpu.COMMIT),
    "precreated_1_2": (
        gpu.BROKEN + "Your worker `w1` exists here. Make a worker `w2` on the remote kernel. Have `w1` fix adder.py "
        "(commit, push); then have `w2` pull, fix muler.py, commit and push (messages: `send` with a `text`; to "
        "`w2` through the bridge). Read `w2`'s `history` back through the bridge. " + gpu.COMMIT +
        "Tell each worker to commit this way."),
    "precreated_tree": (
        gpu.BROKEN + "Your sub-lead `s1` and its worker `w1` exist here. Make a sub-lead `s2` on the remote kernel "
        "and its worker `w2` under it (`parent_id` `s2`). Have `s1` get adder.py fixed through `w1` (commit, push); "
        "then have `s2` get muler.py fixed through `w2` (pull first, commit, push). " + gpu.COMMIT +
        "Tell them to pass it on."),
    "lead_spawns_two": (
        gpu.BROKEN + "Spawn one worker here (`spawn` on `roles`, role `worker`) for adder.py, and one on the remote "
        "kernel for muler.py: make a roles registry there (`create_agent` id `roles`, `handler_module` "
        "`agentic_resources.tools`, `role_dirs` [\"roles\"], `limits` {\"total\": 1}, `tiers` {\"small\": <the "
        "remote AI fields without id and parent_id>}) and `spawn` on it through the bridge. The remote worker "
        "pulls first; each commits and pushes. " + gpu.COMMIT + gpu.DELETE + " Delete the remote registry too."),
    "leads_spawn_leads": (
        gpu.BROKEN + "Spawn a sub-lead here (`spawn` on `roles`, role `sublead`) for adder.py, and one on the remote "
        "kernel for muler.py: make a roles registry there (`create_agent` id `roles`, `handler_module` "
        "`agentic_resources.tools`, `role_dirs` [\"roles\"], `limits` {\"total\": 2}, `tiers` {\"small\": <the "
        "remote AI fields without id and parent_id>}) and `spawn` on it through the bridge. Each sub-lead spawns "
        "one worker on its own kernel; the remote one pulls first; each commits and pushes. " + gpu.COMMIT +
        gpu.DELETE + " Delete the remote registry too."),
    "headless_and_canvas": (
        "Make an agent `rlead` on the remote kernel and have it make a canvas for its notes: a `create_agent` on "
        "its kernel with the id `sketch`, `parent_id` `rlead`, `handler_module` `canvas_page.tools`. Add the line "
        "`local: the notes of the lead` to NOTES.md in your clone (a new file), commit and push; then have `rlead` "
        "pull, add `remote: the notes of rlead`, commit and push. " + gpu.COMMIT + " Keep the remote running "
        "(do not do step 6 yet) and end your run with the `done` call."),
}

PAUSE = ("Delete the bridge and stop the remote kernel as step 6 says, but keep `sketch` and `rlead` on it; "
         "end your run with the `done` call.")

RESTART = ("Your kernel was restarted. Start the remote again: step (1) again (the tunnel and the remote kernel, "
           "in one job); then, in the box, once `~/w/.fantastic/cli/mcp.sock` exists, send `{\"type\":\"boot\"}` "
           f"to `{T.WEB}` (its record is kept, not booted); then step (4), the bridge, again. Make nothing else; "
           "end your run with the `done` call.")

CLEANUP = ("Now clean up as step 6 says: delete `sketch` and `rlead` on the remote kernel through the bridge, "
           "delete the bridge, stop the remote kernel, pull your clone.")


def play(ctx, v, lead: str, text: str, label: str = "") -> gpu.Lead:
    """Hand `lead` its task in plain words and wait for its `done` (as `gpu_cli.play`)."""
    watch = gpu.Lead(ctx.team, lead)
    watch.refresh()  # a lead told before: its earlier `done` does not count
    watch.since = len(watch.rows)
    started = time.time()
    gpu.hand_task(ctx, lead, text + gpu.END)
    how = gpu.wait_done(ctx, watch, max(60.0, ctx.test["timeout"] - gpu.CHECK_MARGIN_S))
    ctx.elapsed = time.time() - started
    v.check(f"{label}the lead answered done", how == "done", f"{how}; {getattr(ctx, 'kicked', {})}")
    return watch
