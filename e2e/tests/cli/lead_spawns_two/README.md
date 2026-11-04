# cli/lead_spawns_two

A lead with a roles registry (fantastic's `agentic_resources`, switched on by
`FANTASTIC_AGENTIC_RESOURCES=1`; the card `roles/worker.md`, the tier `small`) spawns two workers.
They solve the shared fixture, one bug each (`adder.py`, `muler.py`) with a commit apiece, and the
lead deletes them with `delete_agent`.

Start: `--headless`, the blueprint folder imported under `user`
(`fantastic_team import blueprint --parent user --approve`). The cards are not in the blueprint
(a card is code to an import); the probe copies `roles/` next to the kernel, where `role_dirs`
reads it. The workers' tier has no `credentials`, so they run keyless against the scripted
server (a spawned agent is not on the credential store's `callable_by`).

The plug is one script with lanes (`backends/scripted.py`): `main` is the lead, `w1`/`w2` the
workers (their spawn `task` carries `[lane:w1]`); w2 waits for w1, the lead's third turn waits for
both; `{{id:1}}` / `{{id:2}}` in its delete calls are the minted ids from the spawn replies.

Checks: solved; two `spawn` calls; both workers asked the model; the record set after is the record
set before (both workers deleted); the registry counts none running.

Mutation: drop the lead's two `delete_agent` turns from `script.json` and the record set differs.
