# cli/usage_report

The usage report is read from the team on demand (#272), with the surface that exists: the
provider's `usage` verb, sent from the CLI (`fantastic_team send '{"target": "provider",
"payload": {"type": "usage"}}'`). No verb is added.

The test runs single_lead's scenario A (the scripted model, one lead), then reads the provider's
`usage` and checks it against what the scripted plug served: `by_model` holds the model with its
requests and tokens (input, cache write and read, output) and has no `cost_usd` with no prices (`unpriced` when the record's `prices` name other models only);
`by_agent` holds the lead's chat id; the team total is the sum of both. Then `update_agent` gives
the provider a `prices` map and the report carries `cost_usd` per model, per agent and in total.

Mutation: drop a turn from `script.json` and the served count no longer matches; read `usage` from
another provider id and the report is empty.
