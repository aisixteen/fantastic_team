# cli/precreated_1_2

A lead and two workers, all three made with `send` + `create_agent` (the workers are the lead's children,
keyless against the scripted server). Nothing is spawned: the probe makes them under
the fixed ids `lead`, `w1`, `w2`, hands the lead its task, and the lead `send`s each worker its file (`[lane:w1]`,
`[lane:w2]` mark the workers' lanes in the plug). The workers fix one bug each and commit; w2 waits
for w1 (`after`) so the two commits do not meet on the git lock.

Checks: solved (the test passes on HEAD, the fix is a commit); the tree is `lead`, `w1`, `w2`
and the base, nothing else; each sits under the parent it was made under; both workers asked the model; the agents and parents after the run are the ones before it.

Mutation: have the lead `create_agent` anything and "the same agents ... nothing extra" fails;
leave w2's commit out of its lane and "solved" fails.
