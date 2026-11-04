# roles/lead_grants_worker_key

A spawned agent is not on the credential store's `callable_by`, and a spawn with a `task` starts
its first turn at once, so a tier that names `credentials: {store, ref}` is refused (#223). The
card's way, played by the scripted plug against the team binary (headless; the repo's own role
cards; the registry's tiers name the model's key):

1. the main lead `spawn`s a worker WITHOUT `task`;
2. it reads the records with `list_agents` on the root and sends `update_agent` on the root for
   the store's `callable_by`: the whole map it read (`$self` written as the store's own id: a
   call cannot write a `$` tag) plus the worker with `get`;
3. it `send`s the worker its first message; the worker's first turn reads its key and asks the
   model.

The probe then reads the worker's history and deletes it (a lead would delete what it spawned: the delete is the only thing the script leaves out, to read the history first).

Checks: the store names the worker while it lives; every entry the store had stays; the
worker's lane asked the model with the store's key (a refused key read asks nothing); no call was
refused; the worker's history holds its one message and no refusal; after the delete the store is the one it was before and the tree is unchanged.

Fantastic's reach would let the worker read its key a moment after it is made, even with no lead. A first turn started by a spawn's `task` is the one that races it, which is why the spawn has none.

The worker's own `get` on the store is not scripted here: fantastic's reach (a host helper
that gives every AI agent every verb on every agent, a moment after the agent is made) writes
it on the headless binary. `new_team.rs` tests (a kernel without reach) cover both ends.

Mutation: leave the `update_agent` of the store out of the script and the store never names the worker with `get` alone (fantastic's reach writes `*`, which is not what the lead writes), the first check fails.
