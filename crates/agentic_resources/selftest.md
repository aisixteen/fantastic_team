# team-agentic-resources selftest

> scopes: kernel, roles
> requires: `cargo build --release --bin fantastic_kernel`
> out-of-scope: a live model — the roles and the limits are exercised against any bundle
> a spawn's `backend` names

> Moved from fantastic (team#245) as it was there: the steps use fantastic's one-shot
> CLI. In team the bundle runs inside `fantastic_team` (registered by `register_team`,
> no env var needed); send the same verbs with
> `fantastic_team send '{"target": "<id>", "payload": {"type": "<verb>", ...}}'`.

A generic roles registry. A role is a meta bundle (`.fantastic/bundles/<name>/bundle.yaml`:
a `base`, no program, `record.post_system_prompt` and `record.can_call`); the registry's
record carries its `limits`. One-shot CLI: `fantastic <id> <verb> [k=v ...]`; JSON-valued
args are coerced.

## Pre-flight

All test state lives under `/tmp/ar_test/`.

```bash
rm -rf /tmp/ar_test && mkdir -p /tmp/ar_test/.fantastic/bundles/writer && cd /tmp/ar_test
FANTASTIC=/path/to/rust/target/release/fantastic_kernel
cat > .fantastic/bundles/writer/bundle.yaml <<'ROLE'
base: scheduler.tools
sentence: Writes the first draft of a text.
record:
  post_system_prompt: You write clearly and briefly.
ROLE
$FANTASTIC kernel_state reload_bundle name=writer | jq -e '.reloaded == "writer"'
AR=$($FANTASTIC kernel_state create_agent handler_module=agentic_resources.tools | jq -r .id)
```

## Tests

### Test 1: roles lists the role bundles (none is built in)

```bash
$FANTASTIC $AR roles | jq -e '[.roles[].name] == ["writer"]'
```

### Test 2: role returns the full role; an unknown name is not_found

```bash
$FANTASTIC $AR role name=writer | jq -e '.directive == "You write clearly and briefly."'
$FANTASTIC $AR role name=ghost | jq -e '.reason == "not_found"'
```

### Test 3: a cap refuses the spawn past it

```bash
$FANTASTIC $AR set_limit role=writer max=1 | jq -e '.ok'
$FANTASTIC $AR spawn role=writer provider=prov | jq -e '.id'
$FANTASTIC $AR spawn role=writer provider=prov | jq -e '.reason == "limit" and .scope == "role"'
$FANTASTIC $AR get_count role=writer | jq -e '.running == 1 and .max == 1'
```

### Test 4: nothing is cleaned up on done; delete is the only way out

```bash
$FANTASTIC $AR fleet | jq -e '.roles.writer | length == 1'
```

## Cleanup

```bash
$FANTASTIC kernel_state delete_agent id=$AR
rm -rf /tmp/ar_test
```

## Summary table

| Test | Pass / Fail | Notes |
|---|---|---|
| 1. roles | | |
| 2. role / not_found | | |
| 3. limit refuses | | |
| 4. fleet keeps finished | | |
