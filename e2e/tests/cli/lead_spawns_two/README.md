# cli/lead_spawns_two

A lead with a bundles registry (`agentic_resources`, switched on by
`FANTASTIC_AGENTIC_RESOURCES=1`; the bundle `tests/cli/spawn_bundles/worker/bundle.yaml`, shared with `leads_spawn_leads`) spawns two workers.
They solve the shared fixture, one bug each (`adder.py`, `muler.py`) with a commit apiece, and the
lead deletes each with `delete_agent` as soon as it has reported (#264, lead-side: the registry frees nothing on its own).

Start: the lead and its bundles registry laid as record files before the first start of `--headless`,
under the fixed ids `lead` and `bundles`. The probe writes the test's bundles under the team's
`.fantastic/bundles/` and loads them (`reload_bundle`). A spawned worker runs on the lead's backend (its
handler and provider fields) with the lead's model: the spawn names none.

The plug is one script with lanes (`backends/scripted.py`): `main` is the lead, `w1`/`w2` the
workers (the lead's `send` after each spawn carries `[lane:w1]`); w2 waits for w1. A worker's last turn is its report to
the lead, a plain `send` (#247: no `report` verb; it gives the parent a turn, which answers it) and the lead
awaits: each report wakes it, and its turn deletes that worker (the send waits until the lead is idle, so a
worker has no turn after it); the lead's `git log` waits for both lanes; `{{id:1}}` / `{{id:2}}` in its delete calls are the minted ids from the spawn replies.

Checks: solved; two `spawn` calls; both workers asked the model; each report reached the lead and gave it a turn; the record set after is the record
set before (both workers deleted); the registry counts none running.

Mutation: drop the lead's two `delete_agent` turns from `script.json` and the record set differs.
