# roles/app_team_spawn_trees

The team the app makes (`fantastic_team_app --dir`: the cards of `roles/` written next to the
kernel, the agents made by `new_team::steps` with `create_agent`), not test cards, runs two
trees with a scripted model (#227):

- the main lead spawns a sub-lead (the `lead` card), which spawns a worker;
- the main lead spawns a `project_lead`, which spawns a worker.

Each parent deletes what it spawned (a worker by its parent, the sub-leads by the main lead).
The probe only points the model at the plug: the lead's `model`, `endpoint` and credential
ref, and the registry's tiers (model and endpoint, no `credentials`, as in `cli/` fact 2). No
card or grant is touched. The one record change besides that is `limits.total` of the registry,
raised from the app's 3 to 6 with `update_agent`: the two sub-leads and their two workers are
alive at once (4).

Needs `fantastic_team_app` next to `--bin` (a Skip otherwise).

Checks: the cards on disk equal the repo's `roles/`; all four lanes asked the model (so four
spawns succeeded); no agent with `spawned_by` is left; every agent's parent exists; the
records are the ones the app made; the registry counts nothing running.

Mutation: put `kernel: "*"` back in `roles/lead.md` and the sub-lead's spawn is refused
(`not_allowed`); lane `s1` and `w1` never run and the check on lanes fails.
