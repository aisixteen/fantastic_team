# packs/failing_evals_block_enable (free)

Issue #41: a pack whose evals fail cannot be enabled; one whose evals pass can.

Two packs with a blueprint, installed with `packs install --worker lead --parent-id lead`. `badpack` (one passing and one failing eval): the preview (no `--approve`) is not gated; with `--approve` the install exits non-zero with `enable refused`, `enable.allowed` false, nothing imported, no new agent, the template kept. `okpack` (one passing eval) with `--approve`: exit 0, `enable.allowed` true, `installed_as: worker`, and its blueprint's agents are live in `list_agents`.
