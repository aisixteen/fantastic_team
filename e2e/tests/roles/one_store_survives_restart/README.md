# roles/one_store_survives_restart (free, build: any)

Every AI agent's memory and the task registry point at ONE store agent (#185): a `sqlite_store` named `memory`, one file `.fantastic/memory.db` in the team directory. It is config (`team.yaml` may name another store with `store:`); this test runs the default.

The lead, on the scripted model, spawns one worker through the roles registry, then adds one row to `tasks`, then `done`.

Asserts: the agent list holds exactly one `sqlite_store.tools` agent, `memory`, and no `*_memory` agent besides the kernel's own `kernel_memory`; the lead's `memory_store_id`, every tier's `memory_store_id` and `tasks`' `task_store_id` name it; the spawned worker names it too and the store admits the worker; `.fantastic/memory.db` exists in the run dir; after the team is stopped and started again on the same directory, the lead's history rows (`history`) and the `tasks` row (`list`) are back, unchanged.

The scripted model serves the lead only: the worker tier is inert (a RAM store handler), so no worker calls the model.

Coded for a manual run (`python3 e2e/run.py --only roles/one_store_survives_restart`); CI does not run the e2e.
