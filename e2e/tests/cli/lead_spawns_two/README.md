# cli/lead_spawns_two

A lead with a roles registry (`agentic_resources`, switched on by
`FANTASTIC_AGENTIC_RESOURCES=1`; the role bundle `bundles/worker/bundle.yaml`) spawns two workers.
They solve the shared fixture, one bug each (`adder.py`, `muler.py`) with a commit apiece, and the
lead deletes them with `delete_agent`.

Start: `--headless`, then the lead and its roles registry made with `send` + `create_agent`
under the fixed ids `lead` and `roles`. The probe writes the test's role bundles under the team's
`.fantastic/bundles/` and loads them (`reload_bundle`). A spawned worker runs on the lead's backend (its
handler and provider fields) with the lead's model: the spawn names none.

The plug is one script with lanes (`backends/scripted.py`): `main` is the lead, `w1`/`w2` the
workers (their spawn `task` carries `[lane:w1]`); w2 waits for w1, the lead's third turn waits for
both; `{{id:1}}` / `{{id:2}}` in its delete calls are the minted ids from the spawn replies.

Checks: solved; two `spawn` calls; both workers asked the model; the record set after is the record
set before (both workers deleted); the registry counts none running.

Mutation: drop the lead's two `delete_agent` turns from `script.json` and the record set differs.
