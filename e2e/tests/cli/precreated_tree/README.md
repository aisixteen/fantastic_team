# cli/precreated_tree

A tree made with `send` + `create_agent` before the task: a lead, two sub-leads (`s1`, `s2`), and one worker under each (`w1`, `w2`).
The lead `send`s each sub-lead a task, each sub-lead `send`s its worker, the workers fix one bug
each and commit (w2 after w1). Lanes in the plug: `main` is the lead, `s1`/`s2` the sub-leads,
`w1`/`w2` the workers; each lane's marker is in the message that starts it. The lead's last turn
waits for all four lanes.

Checks: solved; the five agents sit under the parents they were made under; the
four non-root agents asked the model; the tree after the run is the tree before it.

Mutation: delete a worker in a script turn and "the same agents ... nothing extra" fails.
