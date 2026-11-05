"""headless_and_canvas: `--headless` opens no port and needs no terminal; `canvas --port N` serves;
`webui --port N` serves the team page and its socket; an agent's `create_agent` of a canvas works
with no `canvas` word. The steps and the checks are `helpers/cli_scenarios.py`'s (the gpu twin uses
them too)."""

from helpers import cli, cli_scenarios as S
from helpers.verdict import Verdict


def run(ctx):
    v = Verdict("cli/headless_and_canvas")
    team = S.canvas_start(ctx, v)
    team.send("lead", {"type": "send", "text": "Make a canvas for your notes."})
    v.check("the scripted model played every turn", cli.wait_served(ctx, 60) and ctx.model.consumed,
            f"served {ctx.model.served} of {len(ctx.model.turns)}")
    S.canvas_end(ctx, v)
    return v.finish()
