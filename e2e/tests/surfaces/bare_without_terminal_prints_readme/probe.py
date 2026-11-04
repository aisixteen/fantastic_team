"""The bare word with no terminal on stdout: fantastic prints its readme and exits 0; it draws
nothing, starts nothing and does not hang. `--tui` is gone: fantastic refuses it as unknown."""

import os
import subprocess

from helpers import cli, team as team_mod
from helpers.verdict import Verdict


def attempt(env, d, args, stdin_pipe):
    os.makedirs(d, exist_ok=True)
    p = subprocess.Popen(args, env=env, cwd=d, stdin=subprocess.PIPE if stdin_pipe else subprocess.DEVNULL,
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        out, err = p.communicate(timeout=20)
        return p.returncode, out.decode(errors="replace"), err.decode(errors="replace"), False
    except subprocess.TimeoutExpired:
        p.kill()
        out, err = p.communicate()
        return None, out.decode(errors="replace"), err.decode(errors="replace"), True


def run(ctx):
    v = Verdict("surfaces/bare_without_terminal_prints_readme")
    env = team_mod.Team(ctx.binary, ctx.run_dir, {}).env
    for name, pipe in (("bare_devnull", False), ("bare_pipe", True)):
        label = "stdin /dev/null" if not pipe else "stdin a pipe"
        d = os.path.join(ctx.run_dir, name)
        code, out, err, hung = attempt(env, d, [ctx.binary], pipe)
        v.check(f"[{label}] it does not hang", not hung)
        v.check(f"[{label}] it exits 0", code == 0, str(code))
        v.check(f"[{label}] it prints fantastic's readme, which names the words",
                "without a terminal" in out and "canvas" in out and "--headless" in out, out[:300])
        v.check(f"[{label}] no terminal UI was drawn", not any(s in out + err for s in cli.ALT_SCREEN))
        v.check(f"[{label}] nothing was started in the folder",
                not os.path.exists(os.path.join(d, team_mod.SOCK)) and not os.path.exists(os.path.join(d, ".fantastic")))
    d = os.path.join(ctx.run_dir, "tui_flag")
    code, out, err, hung = attempt(env, d, [ctx.binary, "--tui"], False)
    v.check("--tui is gone: refused as unknown, exit 2", not hung and code == 2 and "--tui" in err, f"{code} {err[:200]}")
    v.check("--tui started nothing", not os.path.exists(os.path.join(d, ".fantastic")))
    return v.finish()
