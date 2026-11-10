"""gpu_cli_ssh/precreated_tree: `cli_ssh/precreated_tree` on a real model: the lead, s1, w1, s2 and w2 on the master;
the remote asks s2 (a backend on the master) for the box's work; the box and the wiring told in plain words. The
box and the checks are `helpers/ssh_twins.py`'s; the task is `helpers/gpu_ssh.py`'s; no script. The body is
`gpu_ssh.precreated_run`'s."""

from helpers import cli_scenarios as S, gpu_ssh as G


def run(ctx):
    return G.precreated_run(ctx, "precreated_tree", S.TREE, "s2")
