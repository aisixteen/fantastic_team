# team-agentic-resources selftest

> scopes: kernel, bundles
> requires: `cargo build --release --bin fantastic_kernel`
> out-of-scope: a live model — the bundles are exercised against any bundle

> Moved from fantastic (team#245) as it was there: the steps use fantastic's one-shot
> CLI. In team the bundle runs inside `fantastic_team` (registered by `register_team`,
> no env var needed); send the same verbs with
> `fantastic_team send '{"target": "<id>", "payload": {"type": "<verb>", ...}}'`.

A generic bundles registry. A spawnable bundle is a meta bundle (`.fantastic/bundles/<name>/bundle.yaml`:
a `base`, no program, `record.post_system_prompt` and `record.can_call`). One-shot CLI: `fantastic <id> <verb> [k=v ...]`; JSON-valued
args are coerced.

## Pre-flight

All test state lives under `/tmp/ar_test/`.

```bash
rm -rf /tmp/ar_test && mkdir -p /tmp/ar_test/.fantastic/bundles/writer && cd /tmp/ar_test
FANTASTIC=/path/to/rust/target/release/fantastic_kernel
cat > .fantastic/bundles/writer/bundle.yaml <<'BUNDLE'
base: scheduler.tools
sentence: Writes the first draft of a text.
record:
  post_system_prompt: You write clearly and briefly.
BUNDLE
$FANTASTIC kernel_state reload_bundle name=writer | jq -e '.reloaded == "writer"'
AR=$($FANTASTIC kernel_state create_agent handler_module=agentic_resources.tools | jq -r .id)
```

## Tests

### Test 1: bundles lists the spawnable bundles (none is built in)

```bash
$FANTASTIC $AR bundles | jq -e '[.bundles[].name] == ["writer"]'
```

### Test 2: bundle returns the full card; an unknown name is not_found

```bash
$FANTASTIC $AR bundle name=writer | jq -e '.directive == "You write clearly and briefly."'
$FANTASTIC $AR bundle name=ghost | jq -e '.reason == "not_found"'
```

### Test 3: spawn makes an agent on the bundle and get_count reads the tree

```bash
$FANTASTIC $AR spawn bundle=writer provider=prov | jq -e '.id'
$FANTASTIC $AR get_count bundle=writer | jq -e '.running == 1'
```

### Test 4: nothing is cleaned up on done; delete is the only way out

```bash
$FANTASTIC $AR fleet | jq -e '.bundles.writer | length == 1'
```

## Cleanup

```bash
$FANTASTIC kernel_state delete_agent id=$AR
rm -rf /tmp/ar_test
```

## Summary table

| Test | Pass / Fail | Notes |
|---|---|---|
| 1. bundles | | |
| 2. bundle / not_found | | |
| 3. spawn and count | | |
| 4. fleet keeps finished | | |
