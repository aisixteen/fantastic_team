#!/bin/sh
# One AI worker, made the way fantastic makes any agent: `create_agent` on the root,
# through `fantastic_team send`, under an id you choose. There is no blueprint and no import.
#
#   fantastic_team --headless &        # a team running in this folder
#   sh examples/create_agent/worker_only.sh
#
# then give it work: fantastic_team send '{"target": "w1", "payload": {"type": "send", "text": "..."}}'
#
# A grant is written on BOTH ends (a new agent is born with `$self` only): the worker's own
# `can_call` and the other agent's `callable_by`. The agent you create carries its own end in
# the payload; the other end is on an agent that exists already (the home's `memory` and
# `credentials`), so it is a second `update_agent`. `update_agent` REPLACES a whole map:
# restate the keys the agent has (read them with `list_agents`, shown first), then add yours.
#
# The model, endpoint, prices and grants are the record's own fields; edit them later with
# `update_agent`. A key is never written here: `credentials` names a ref in the credential store.
set -e
T=${FANTASTIC_TEAM:-fantastic_team}

# 1. the home's stores: what `memory` and `credentials` admit now
$T send '{"target": "kernel_state", "payload": {"type": "list_agents"}}'

# 2. the worker
$T send '{"target": "kernel_state", "payload": {"type": "create_agent", "id": "w1", "parent_id": "user", "handler_module": "anthropic_backend.tools", "model": "example-model", "endpoint": "https://api.example.com/v1/messages", "credentials": {"store": "credentials", "ref": "EXAMPLE_KEY"}, "price_input_per_mtok": 1.0, "price_output_per_mtok": 5.0, "memory_store_id": "memory", "can_call": {"$self": "*", "memory": "*", "credentials": "*"}, "callable_by": {"$self": "*", "$parent": "*"}}}'

# 3. the other ends. `memory` and `credentials` keep what step 1 showed (`user` for `memory`)
$T send '{"target": "kernel_state", "payload": {"type": "update_agent", "id": "memory", "callable_by": {"$self": "*", "user": "*", "w1": "*"}}}'
$T send '{"target": "kernel_state", "payload": {"type": "update_agent", "id": "credentials", "callable_by": {"$self": "*", "w1": "*"}}}'
