# gpu_cli/headless_and_canvas

The gpu twin of `cli/headless_and_canvas` (#242): the surfaces of the command line, and the canvas as an
agent's own subagent, made by a real model (the shared vLLM server) instead of a script.

1. `fantastic_team --headless`, a lead made with `create_agent` that may `create_agent` on the root. The probe
   tells the lead in plain words (`helpers/gpu_cli.py TASKS`) to make a canvas for its notes: `create_agent` on
   `kernel` with the id `sketch`, under itself, `handler_module: canvas_page.tools`, then `done`. Checks: the
   canvas is there with no `canvas` word, no web host, no TCP port, no terminal UI.
2. `canvas --port N`, 3. `webui --port N`, 4. both refused together, 5. `--headless` again with no port: the
   same checks as the cli test (`helpers/cli_scenarios.py canvas_end`).

Waits for the lead's `done` (or the test's `timeout`). Which call the model made first and how many turns it
took are not asserted; the log has a `report:` line for them. Nothing is spawned, so this test does not depend
on aisixteen/fantastic#467.

Needs `VLLM_ENDPOINT`, `FANTASTIC_AI_MODEL`, `VLLM_KV_SLOTS` (1 or 2) and `VLLM_KV_OVERFLOW`; without one the
test is `SKIP`. Skips too when neither `lsof` nor `/proc/net/tcp` can list listeners. See `e2e/README.md`,
"The gpu tier".

Run: `python3 e2e/run.py --only gpu_cli/headless_and_canvas --bin ABS_PATH`
