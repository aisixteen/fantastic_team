# packs/enable_needs_bindings (free)

Install with `parent_id` and `approve: true`. No bindings: refused with `enable refused: ...` naming `connector:github`, `enable_error` set, the template kept, nothing created. Bound to an agent that does not exist: refused, the message names the port and not the id. Bound to the live `github` agent: the worker is built. The scripted model is never called.

Mutation: make `enable_gate` return `Ok(())` first and the import refuses instead, with no `enable_error`.
