"""gpu_cli/headless_and_canvas: the lead, told in plain words, makes a canvas with `create_agent` on a
real model; then the surfaces: `--headless` opens no port and needs no terminal, `canvas --port N`
serves, `webui --port N` serves the team page and its socket, the two together are refused.
The steps and the checks are `helpers/cli_scenarios.py`'s, the same as `cli/headless_and_canvas`; no script."""

from helpers import cli_scenarios as S, gpu_cli as gpu
from helpers.verdict import Verdict


def run(ctx):
    v = Verdict("gpu_cli/headless_and_canvas")
    S.canvas_start(ctx, v)
    watch = gpu.play(ctx, v, "lead", "headless_and_canvas")
    gpu.lead_on_default(ctx, v)
    gpu.report(ctx, watch)  # before the restarts below: the plug reads the lead's usage from this team
    S.canvas_end(ctx, v)
    gpu.check_key_not_on_disk(ctx, v)
    return v.finish()
