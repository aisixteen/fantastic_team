# packs/install_blueprint_approve (free)

Install with `parent_id` and `bindings {connector:github: github}`. Without `approve` the reply is the permission preview and no agent exists; the binding is kept in `bindings.json`. With `approve: true` the blueprint is built: `installed_as` is `worker`, `ids.scribe` is `scribe_1`, the port is bound to `github` and the live record names `github` instead of `@connector:github`. The scripted model is never called.

Mutation: pass `approve: true` to `import_subtree` unconditionally and the preview creates `scribe_1`.
