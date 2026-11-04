# roles/sublead_worker_key_via_lead

A sub-lead holds no grant on the credential store and no `update_agent` (#223). Its worker's key
comes from the main lead. The scripted plug plays three agents against the headless binary
with the repo's own role cards (tiers name the model's key):

1. the main lead spawns a sub-lead (the `lead` card) without a task, restates the store's
   `callable_by` with the sub-lead added (`$self` written as the store's id), and sends the
   sub-lead its first message;
2. the sub-lead spawns a worker without a task and asks the main lead: `send` to `lead`,
   `need_key: <the worker's id> ...`;
3. the main lead's next chain restates the store's map with the worker added (the id from the
   ask, by the plug's `{{tag:need_key}}`) and sends the worker its first message; the worker's first
   turn reads its key and asks the model.

Checks: both agents are on the store with `get`; every entry the store had stays; the main
lead, the sub-lead and the worker all asked the model with the store's key; no call was
refused; the main lead's request holds the sub-lead's ask naming the worker; neither history
has a refusal; the probe deletes the sub-lead (and its worker) and the store is the one it was before.

The edge `lead` <- sub-lead (`send`) and the agents' own `get` on the store are not scripted:
fantastic's reach writes them on this binary. The `new_team.rs` test
`a_sub_leads_worker_gets_its_key_through_the_main_lead` runs the whole card on a kernel without
reach, where the sub-lead is refused each of them until the main lead grants it.

Mutation: leave the second `update_agent` out of the script and the worker is on the store with
`*` (reach) and never `get` alone: the first check fails.
