# roles/app_team_spawn_trees

The team the app makes (`fantastic_team_app --dir`: the agents seeded by the first run from `crates/team/blueprint.yaml`), not test cards, runs two
trees with a scripted model (#227):

- the main lead spawns a sub-lead (the `lead` card), which spawns a worker;
- the main lead spawns a `project_lead`, which spawns a worker.

Each parent deletes what it spawned (a worker by its parent, the sub-leads by the main lead).
The key mode is the person's choice: the probe chooses `plain` as the set-up page does (`set_key_mode` on a `setup.tools` agent), so fantastic makes the `credentials` store; the app and the team choose none (#261). The probe keeps the model's key in it with the store's `set` (the ref declared first), as the set-up page does. The probe then only points the model at the plug: it adds a provider under the lead (its key the one in the credential store) and the lead names it (`provider_id`, `model`), as the team's yaml says the person does. No
card or grant is touched. The one record change besides that is `limits.total` of the registry,
raised from the app's 3 to 6 with `update_agent`: the two sub-leads and their two workers are
alive at once (4).

Needs `fantastic_team_app` next to `--bin` (a Skip otherwise).

Checks: the roles registry serves the repo's role bundles (`crates/team/bundles/`); all four lanes asked the model (so four
spawns succeeded); no agent with `spawned_by` is left; every agent's parent exists; the
records are the ones the app made; the registry counts nothing running.

Mutation: put `kernel: "*"` back in the `lead` bundle's `can_call` (`crates/team/bundles/lead/bundle.yaml`) and the sub-lead's spawn is refused
(`not_allowed`); lane `s1` and `w1` never run and the check on lanes fails.

Waiting on aisixteen/fantastic#467 (#233): under fantastic's reach (god mode) every agent is granted `*` on the credential store, so no key procedure exists in the team. But reach writes a new agent's grants after a spawn with a task has begun its first turn. With the tiers' `credentials` ref kept, that first turn fails and the spawned agents never answer (checked: 3 of 13 turns served). So this test removes the tiers' `credentials` ref (the spawned agents ask the plug without a key), and the check "a worker spawned with a task reads its key under reach" is NOT made here until #467 is fixed. No sleep, no grant in the probe.
