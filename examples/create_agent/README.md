# Creating agents with `send`

A team is agent records, and a record is made the way fantastic makes any agent: a
`create_agent` payload sent to the root with `fantastic_team send`. There is no blueprint and no
import. You choose the id, so a role card that grants `github` or `queue` by id stays true as
written (fantastic's `import` gives every agent a `_1` suffix, which is why the team does not use
it).

- [worker_only.sh](worker_only.sh): one AI worker with its grants on both ends. Run it in the
  folder of a running team (`fantastic_team --headless`).
- The desktop app makes its own team the same way (`crates/team/src/new_team.rs`): the lead
  `lead` and, under it, `queue`, `github`, `board`, `roles`, `tasks`, `worker_exec` and
  `worker_files`. Its steps are plain `create_agent` payloads; read them there or in the units
  (`new_team::tests`).

## The shape of a payload

```json
{"target": "kernel_state", "payload": {"type": "create_agent", "id": "<id>", "parent_id": "<parent>",
  "handler_module": "<bundle>", "can_call": {"$self": "*"}, "callable_by": {"$self": "*", "$parent": "*"}}}
```

- `target` is the root (`kernel_state`, or the alias `kernel`).
- `parent_id` is where the agent sits in the tree (`user` for a home agent, a lead for its tools).
- `can_call` is what the new agent may call; `callable_by` is who may call it. An edge needs
  both ends: when the other agent exists already, restate its map with `update_agent` and add
  the key (`update_agent` replaces the whole map).
- Ports of a blueprint (`@memory`) do not exist here: name the agent (`"memory_store_id": "memory"`).
