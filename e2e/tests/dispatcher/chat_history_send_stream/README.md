# dispatcher/chat_history_send_stream (free)

Against a live team, the page's chat verbs `history`, `send` and `status` work over a websocket, and the answer streams to a watcher as `part_start`, `part_delta`, `part_end` and a `done` status.

The team has one lead on the scripted model, in the `team` group, started with `--port`. The binary's `--port` serves the canvas, whose socket leg is `canvas_ws` (the page's own leg, `team_ws`, is served by the in-process `TeamHandle`, whose registry has no model backend, so it cannot host a lead). The harness gives `canvas_ws` the grant `tree::grant_chat_to` writes for the page's leg: `history`, `send`, `stop`, `status`, `watch` on the lead, both ends, by an operator write (`helpers/team.py grant_chat`).

Asserts: `history` is empty and `status` is idle before; `send` is accepted; the watcher gets a text part for its `client_id` whose deltas carry the scripted text exactly once, then `part_end` and a done status; `history` then holds the user text and the assistant text; `status` is idle. A tool outside the grant (`worker_exec`) and a verb outside the chat verbs (`reflect` on the lead) are refused, so the page reaches only what it is granted.

The runtime batches a short answer into one `part_delta`; the check is "at least one delta, the text once".

Mutation: remove the `grant_chat` call in `helpers/run_flow.boot_page`; the first `history` call is refused and the test fails.
