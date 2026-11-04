# cli/leads_spawn_leads

A lead (with the roles registry, `FANTASTIC_AGENTIC_RESOURCES=1`) spawns two sub-leads (the card
`sublead`), each sub-lead spawns a worker (the card `worker`), the workers fix one bug each and
commit (w2 after w1), and each parent deletes what it spawned: a sub-lead its worker, the lead its
two sub-leads. Cleanup is the model's decision (`delete_agent`), as in fantastic: no kernel or team
rule does it; here the scripted model decides.

Four lanes plus the lead's (`s1`, `s2`, `w1`, `w2`): a spawn's `task` carries the marker. The lead's
delete turns wait for every lane (`after`), so a sub-lead is never deleted while it still has a
turn to play; `{{id:N}}` in a delete call is the minted id from the Nth spawn reply the agent saw.

Checks: solved; four `spawn` calls; every lane asked the model; no record with `spawned_by` is left;
every agent's parent exists; the agents and parents after are the ones before; the registry counts
none running.

Mutation: drop a sub-lead's delete turn and "no spawned agent is left" fails.
