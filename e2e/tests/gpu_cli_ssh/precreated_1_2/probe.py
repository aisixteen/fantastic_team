"""gpu_cli_ssh/precreated_1_2: `cli_ssh/precreated_1_2` on a real model: the lead, w1 and w2 on the master; the
remote asks w2 (a backend on the master) for the box's work; the lead is told the box and the wiring in plain
words. The box and the checks are `helpers/ssh_twins.py`'s; the task is `helpers/gpu_ssh.py`'s; no script. The
body is `gpu_ssh.precreated_run`'s."""

from helpers import cli_scenarios as S, gpu_ssh as G


def run(ctx):
    return G.precreated_run(ctx, "precreated_1_2", S.ONE_TWO, "w2")
