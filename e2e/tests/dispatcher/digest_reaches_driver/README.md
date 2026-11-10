# dispatcher/digest_reaches_driver (free)

Roles mode (the test's own fleet/agents/roles/agent.yaml; the roles registry, the task registry, the cadencer reading the spawned tree from them). The probe spawns a `project_lead` through the registry as the operator and hands it one message; its scripted turns spawn two workers through `roles`. The probe then sets the cadencer's cadence to 2s; it never sends a `tick`.

Asserts: a model request reaches the lead holding the digest (both spawned workers, named as the registry's `fleet` lists them, as `<id>: idle`, then the `load:` line); the cadencer's `status` shows `ticks` and `digests_sent` of at least 1, `delivery_errors` 0 and a metered `digest_bytes_per_hour`; the team log has no `cadencer call tick`.

The digest tree is the one under the registry's `project_lead` (`crates/team/src/board.rs` `roles_tree_rows`), so the scripted lead is the spawned `project_lead`, not the stock `lead`.

Mutation: return early in `restart_timer` (`crates/team/src/cadencer.rs`) before the timer is spawned and the test fails.
