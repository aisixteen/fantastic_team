"""`--tui` with no terminal on stdout: the binary refuses at once, non-zero, with
a message naming the alternatives; it draws nothing, starts nothing, and does not hang."""

import os
import subprocess

from helpers import team as team_mod
from helpers.verdict import Verdict

ALT_SCREEN = ("\x1b[?1049h", "\x1b[?25l", "\x1b[2J")


def attempt(ctx, env, name, stdin_pipe):
    d = os.path.join(ctx.run_dir, name)
    cmd = [ctx.binary, "--dir", d, "--tui"]
    p = subprocess.Popen(cmd, env=env, stdin=subprocess.PIPE if stdin_pipe else subprocess.DEVNULL,
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        out, err = p.communicate(timeout=20)
        return d, p.returncode, out.decode(errors="replace"), err.decode(errors="replace"), False
    except subprocess.TimeoutExpired:
        p.kill()
        out, err = p.communicate()
        return d, None, out.decode(errors="replace"), err.decode(errors="replace"), True


def run(ctx):
    v = Verdict("surfaces/tui_without_terminal_refuses")
    env = team_mod.Team(ctx.binary, ctx.run_dir, {}).env
    for name, pipe in (("tui_devnull", False), ("tui_pipe", True)):
        label = "stdin /dev/null" if not pipe else "stdin a pipe"
        d, code, out, err, hung = attempt(ctx, env, name, pipe)
        v.check(f"[{label}] it does not hang", not hung)
        v.check(f"[{label}] it exits 2", code == 2, str(code))
        v.check(f"[{label}] the message says --tui needs a terminal", "--tui needs a terminal" in err, err[:300])
        v.check(f"[{label}] the message names the alternatives", "no flags" in err and "--port N" in err)
        v.check(f"[{label}] no terminal UI was drawn", not any(s in out + err for s in ALT_SCREEN))
        v.check(f"[{label}] nothing was started in the dir",
                not os.path.exists(os.path.join(d, team_mod.SOCK)) and not os.path.exists(os.path.join(d, ".fantastic")))
    return v.finish()
