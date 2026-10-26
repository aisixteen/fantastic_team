# dispatcher/chat_features (free)

One live turn of the page's chat with reasoning and a tool call, against the real binary and the scripted model, over a websocket.

Same setup as `chat_history_send_stream` (a lead in the `team` group, `--port`, `canvas_ws` given the page's chat grant by the harness; `watch` on `queue` too, because a tool call part is sent to the watchers of its addressee). The script: turn 1 thinks (`thinking`), says "checking" and calls `queue` `read`; turn 2 waits 3 s (`delay`), says "done" and ends the chain; turn 3 answers a send that carries an attachment.

Asserts, from what the live verbs and events return:
- `status` mid-turn (asked from a second socket) says the turn is generating, is the page's own (`is_mine`) and gives its phase; it is idle after.
- a `thinking` part streams (`part_start`, `part_delta` with the thought, `part_end`);
- the tool call is a `call` part that names its target (`queue`) and carries the verb in its text (`<type>read</type>`), and the queue got the verb;
- the final text part is `done`;
- the done status carries the context numbers (`context_window`, `prompt_tokens`);
- `history` replays the reasoning (`has_thinking`), the call and its reply;
- the `user` agent answers `inquiries` for the lead (none open: the harness has no model that asks);
- an attachment travels as text inside the `send` (`[attached: NAME]` and the content) and reaches the model.

Not asserted: the page's `status` `tool_calling` event. The Rust ai-core at the pinned fantastic never emits it (only the Python runtime does), so a tool block drawn from it has no live source here; the call part above is what a live team sends. Inquiry cards are not driven: no scripted turn asks a question.

Mutation: remove `"thinking"` from `script.json`; the reasoning check fails. Remove the `delay`; the mid-turn status check fails.
