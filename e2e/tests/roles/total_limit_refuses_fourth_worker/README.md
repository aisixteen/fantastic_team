# roles/total_limit_refuses_fourth_worker (free, build: any)

A team with `roles: limits: total: 3` (#185). The lead, on the scripted model, sends the roles agent four `spawn {role: worker}` calls, ONE call per turn (calls in a turn run at the same time and would race the limit), then `done`.

The cards come from the repo's `roles/` folder (the probe copies it into the run dir, where `roles.dirs: [roles]` finds it). The worker and issue_keeper tiers are inert (a RAM store), so no spawned worker calls the model: the single scripted model stays the lead's, in order.

Asserts: `get_count` is `{running: 3, max: 3}`; `fleet` lists 3 workers; the lead's fourth tool result, in the next model request, is the refusal `{error: "limit"}` with the reason "all roles together are at the limit (3 running, max 3)", and the third was not refused; the dispatcher's sidebar carries `fleet` `{running: 3, max: 3}`.

Not covered here: the credential store admits only the lead, so a spawned worker on a real model tier cannot read its key until the operator names it (see the README of the roles section).
