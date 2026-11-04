# cli/precreated_1_2

A lead and two workers, all three in the blueprint folder (the workers are the lead's children,
keyless against the scripted server). Nothing is spawned: the probe imports the blueprint under
`user`, hands the lead its task, and the lead `send`s each worker its file (`[lane:w1]`,
`[lane:w2]` mark the workers' lanes in the plug). The workers fix one bug each and commit; w2 waits
for w1 (`after`) so the two commits do not meet on the git lock.

Checks: solved (the test passes on HEAD, the fix is a commit); the import made `lead_1`, `w1_1`,
`w2_1` (fantastic renames every imported id with `_1`); each sits under the parent the blueprint
gave it; both workers asked the model; the agents and parents after the run are the ones before it.

Mutation: have the lead `create_agent` anything and "the same agents ... nothing extra" fails;
leave w2's commit out of its lane and "solved" fails.
