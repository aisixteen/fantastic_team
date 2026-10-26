# roles/project_lead_plans_and_runs_issues (free, build: any)

The coder `project_lead` (#185, extended) over 4 fake issues, on the scripted model and the fake forge: "do your best choice", a proposed tree, then "yes", two workers are started through the registry, a PR is opened for each, and the tree stays.

How it runs: the team has a lead (unused). The probe copies the repo's `roles/` folder into the run dir, then `spawn`s `project_lead` through the roles agent as the operator, names it on the credential store, the connector, the board and the queue, and chats to it.

## Ordering assumptions (the scripted plug is ONE ordered script for all agents)

Only one agent has a model request open at a time, so the script's order is the run's order:

1. The project_lead reads (`github issues`, `board rows`, `roles get_count`, in one turn: tool calls, not model requests), then answers with the proposal and a `done` call, which ends its run. A plain text turn would not: an agent answers its own reply once more, a request that would take the next script turn. No worker exists yet.
2. After the probe says "yes" it spawns worker 1 and ends its run again. The worker's first message is refused on its key (the `small` tier names a ref the store does not hold), so it asks the model nothing. This makes the order certain: a key read that is refused only by `callable_by` was seen to succeed now and then, and a worker that asked the model at once would take the project_lead's next script turn.
3. The probe then plays the lead's admit step: it gives the worker the real ref and names it in the credential store, connector and queue `callable_by`, and sends it its task again. The worker plays its two turns (`report done`, a closing line) alone.
4. The probe sends the project_lead "continue"; it spawns worker 2 the same way; a last "continue" gets the plan updated in place.

## Asserts

Before "yes": two model requests, `fleet` empty, only the project_lead running. The proposal names all four issues and is still in the project_lead's next request. A worker asked the model nothing before it was admitted. After: the fake forge holds two draft PRs from `team/issue-1` and `team/issue-2`, `fleet` lists both workers by id and the log shows their two `report` sends, the project_lead and both workers are still in the tree, no `delete_agent` was sent, and the last answer ticks the plan in place. The project_lead's reads show in the team's log.

## Skipped, with a note

The chat entry in the dispatcher, the plan shown and updated in place, and the approval gate are LATER (UI) items of #185: the probe chats through the agent's own `send`, and the plan is the project_lead's reply text.

## Known gaps this test works around

- `title` is a routing field of a model call (ai_core lifts it out of the arguments), and the github connector's `pr_create` takes a `title`: a worker's own call cannot deliver it and is answered `pr_create: title required`. So the probe opens each PR through the connector as the operator once the worker has reported. The fix belongs to the connector or to ai_core, not to this test.
- Spawn does not open the credential store or the tools' `callable_by` to the new agent, and only the tools' parent (the lead) or the operator can write them. The project_lead has no `kernel` grant, so a sub-lead cannot admit its own workers: the probe does it as the operator.
- The worker card (the old worker text) grants no connector; a team whose workers open PRs gives them `github` on its own worker card.
