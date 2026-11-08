"""The gpu tier's ssh twins (#242, #252): the six cli scenarios split across the local kernel (the master) and
one in a Podman box, on a real model (the vllm plug, no script).

The box, the fixture on the test's git server and the checks are `helpers/ssh_twins.py`'s, the
same as the scripted twins'. Here the lead is told in plain words where the box is and what the wiring is
(host, port, key path, the login, the tunnel's port, the git server's URL as the box dials it, the master's
guid); it writes every command itself. Nothing asserts the model's own path.

All the backends are on the master and so is the model's key (a keyed server is no longer a SKIP): the box
holds tools, and a remote agent that wants a model sends the master's backend its work with `send` and
`watch`es it, `<master guid>|<backend>`, over the bridge (the box's work program). The vllm plug counts the
usage of the local kernel's agents, which are all of them.
"""

from __future__ import annotations

import json
import time

from . import cli, gpu_cli as gpu, podman_remote as pr, ssh_twins as T


def require(ctx) -> None:
    pr.require()


def wiring(ctx, box: T.Box, backends=()) -> str:
    """The remote and the weak wiring, in plain words, with this run's values. `backends`: the master's
    backends the remote is to call (named in the bridge's `can_call`)."""
    vals = box.values()
    guid = vals["master_guid"]
    ssh = " ".join(pr.ssh_argv(vals["ssh_port"], vals["ssh_key"]))
    start = T.kernel_start_command()
    exec_rec = {k: v for k, v in T.REMOTE_EXEC.items() if k != "id"}
    exec_rec["can_call"] = {"$self": "*", f"{guid}|*": T.BACKEND_VERBS}
    install = T.work_install_command()
    bridge = dict(T.bridge_record(backends), port=int(vals["tunnel_port"]))
    return (
        "THE REMOTE. A Linux box you reach with ssh only: `" + ssh + "` (login `team`, host 127.0.0.1, port "
        f"{vals['ssh_port']}, key {vals['ssh_key']}). It has `fantastic_team`, git and python3, and NO model: no AI "
        "agent runs there and it holds no key; all AI agents are yours, on this kernel. Your exec agent "
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
        f"\"send\"]`; the exec agent `{cli.EXEC}` there: `{json.dumps(exec_rec)}` (its `can_call` lets it call "
        f"your kernel's agents, `{guid}|<agent>`, `send`, `watch`, `status` and `stop` only); the remote work "
        f"program, one command in `~/w`: `{install}`; then send `{{\"type\":\"boot\"}}` to `{T.WEB}`. "
        f"(3) In the box: `cd ~/w && git clone {vals['remote_git_url']} {T.REPO}` (the box's git config "
        "carries the git server's credential). "
        f"(4) On your kernel, `create_agent` the bridge: `{json.dumps(bridge)}`. Its leg is open, and may call "
        f"exactly `send`, `watch`, `status` and `stop` on the backends named in its `can_call`; each of those "
        f"backends admits it (`callable_by` `{T.BRIDGE}`). "
        f"(5) A call on the remote kernel is `forward` on `{T.BRIDGE}` with `target` `user` and `payload` "
        "`{\"type\":\"forward\",\"target\":\"<remote agent id>\",\"payload\":{<the call>}}`; its reply is the "
        "remote's answer. THE REMOTE'S MODEL WORK: no agent on the remote makes a model call; a remote agent "
        f"asks one of your backends. The work program does it: `forward` to `{cli.EXEC}` there, `run` of `python3 "
        f"../work.py {guid} <backend id> <label> <the work, in words>` (it `watch`es `{guid}|<backend id>`, `send`s "
        "it the work, notes the events of the backend's turn and exits 0 when its first pass is done; the backend "
        "runs the turn here and does the work in the box through `forward` on the bridge to the remote exec agent). "
        "A backend you make or spawn after the bridge exists (a new id) must be opened to the leg before the "
        f"remote may call it: `update_agent` of `{T.BRIDGE}` with its `can_call` plus `<id>`: {json.dumps(T.BACKEND_VERBS)}, "
        f"and of `<id>` with its `callable_by` plus `{T.BRIDGE}`: the same four verbs (read the records first, "
        "update replaces the whole map). "
        "(6) At the end: delete every agent you made on the remote kernel for the work (through the bridge), "
        f"but KEEP the remote's surface (`{T.WEB}`, `{T.LEG}`) and its exec agent (`{cli.EXEC}`): they stay, "
        f"the remote is set up for the next run. Then delete the "
        f"bridge, stop the remote kernel with `ssh ... '{T.stop_command()}'` (the binary has no stop word; "
        "the tunnel job ends with it), and pull your local clone (`git pull --ff-only`). "
        "Both clones push to the same git server: `git push origin HEAD:main`; pull before you change a file "
        "the other side pushed. ")


TASKS = {
    "single_lead": (
        gpu.BROKEN + "Fix adder.py in your local clone, commit and push; then, in your workspace on the remote "
        f"box (the remote kernel's `{cli.EXEC}`, through the bridge), pull, fix muler.py, commit and push. " + gpu.COMMIT),
    "precreated_1_2": (
        gpu.BROKEN + "Your workers `w1` and `w2` exist here. Have `w1` fix adder.py (commit, push; `send` with a "
        "`text`). Then have the remote ask `w2` (the work program, step 5; `w2` already admits the bridge): the work "
        "is to pull, fix muler.py, commit and push in the remote workspace, where `w2`'s commands go through "
        f"`forward` on `{T.BRIDGE}` to the remote `{cli.EXEC}`: say so in the work. Read `w2`'s `history`. " + gpu.COMMIT +
        "Tell each worker to commit this way."),
    "precreated_tree": (
        gpu.BROKEN + "Your sub-leads `s1` and `s2` exist here, with their workers `w1` under `s1` and `w2` under `s2`. "
        "Have `s1` get adder.py fixed through `w1` (commit, push). Then have the remote ask `s2` (the work program, "
        "step 5; `s2` already admits the bridge): the work is to get muler.py fixed through `w2` (pull first, commit, "
        f"push), whose commands go through `forward` on `{T.BRIDGE}` to the remote `{cli.EXEC}`: say so in the work. "
        "Read `w2`'s `history`. " + gpu.COMMIT + "Tell them to pass it on."),
    "lead_spawns_two": (
        gpu.BROKEN + "Spawn two workers here (`spawn` on `roles`): `w1` for adder.py, role `worker`, with that task, "
        "and one for the remote workspace, role `remote_worker` (it has no local exec), with no task. Open the second to the leg (step 5), then have the remote "
        "ask it (the work program): the work is to pull, fix muler.py, commit and push in the remote workspace, "
        f"where its commands go through `forward` on `{T.BRIDGE}` to the remote `{cli.EXEC}`: say so in the work. "
        "Each commits and pushes. " + gpu.COMMIT + gpu.DELETE),
    "leads_spawn_leads": (
        gpu.BROKEN + "Spawn two sub-leads here (`spawn` on `roles`): one for adder.py, role `sublead`, with that task, "
        "and one for the remote workspace, role `remote_sublead`, with no task. Open the second to the leg (step 5), then have the remote "
        "ask it (the work program): the work is to have a worker pull, fix muler.py, commit and push in the remote "
        f"workspace, whose commands go through `forward` on `{T.BRIDGE}` to the remote `{cli.EXEC}`: say so in the "
        "work, for the sub-lead to pass on to its worker word for word. Each sub-lead spawns one worker (the remote "
        "sub-lead's with role `remote_worker`, which has no local exec), and deletes it when done. " + gpu.COMMIT + gpu.DELETE),
    "headless_and_canvas": (
        "Your agent `rlead` exists here (it admits the bridge). Have the remote ask it (the work program, step 5): "
        "the work is to make a canvas for its notes, a `create_agent` on the remote kernel (through the bridge) with "
        "the id `sketch`, `parent_id` `user`, `handler_module` `canvas_page.tools`, then to pull in the remote "
        "workspace, add the line `remote: the notes of rlead` to NOTES.md there, commit and push (its commands go "
        f"through `forward` on `{T.BRIDGE}` to the remote `{cli.EXEC}`). Add the line "
        "`local: the notes of the lead` to NOTES.md in your clone (a new file), commit and push before that. "
        + gpu.COMMIT + " Keep the remote running "
        "(do not do step 6 yet) and end your run with the `done` call."),
}

PAUSE = ("Delete the bridge and stop the remote kernel as step 6 says, but keep `sketch` on it; "
         "end your run with the `done` call.")

RESTART = ("Your kernel was restarted. Start the remote again: step (1) again (the tunnel and the remote kernel, "
           "in one job); then, in the box, once `~/w/.fantastic/cli/mcp.sock` exists, send `{\"type\":\"boot\"}` "
           f"to `{T.WEB}` (its record is kept, not booted); then step (4), the bridge, again. Make nothing else; "
           "end your run with the `done` call.")

CLEANUP = ("Now clean up as step 6 says: delete `sketch` on the remote kernel through the bridge, "
           "delete the bridge, stop the remote kernel, pull your clone.")


def play(ctx, v, lead: str, text: str, label: str = "") -> gpu.Lead:
    """Hand `lead` its task in plain words and wait for its `done` (as `gpu_cli.play`)."""
    watch = ctx.watch = gpu.Lead(ctx.team, lead)
    gpu.wait_booted(ctx, watch)
    watch.mark()  # a lead told before: its earlier `done` does not count
    started = time.time()
    gpu.hand_task(ctx, lead, text + gpu.END)
    how = gpu.wait_done(ctx, watch, max(60.0, ctx.test["timeout"] - gpu.CHECK_MARGIN_S))
    ctx.elapsed = time.time() - started
    v.check(f"{label}the lead answered done", how == "done", f"{how}; {getattr(ctx, 'kicked', {})}")
    return watch
