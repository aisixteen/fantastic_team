# ui/sidebar_opens_agent_full_screen (free)

The page's socket, end to end against the real binary: one group `fantastic` (no driver, no board) with two workers, `w1` and `w2`, and two subagents of `w2` (`sub_a`, `sub_b`: kernel agents whose parent is `w2`), all on the scripted model. The model's script is two `done` turns, one per message sent.

The page is two empty columns for now (see `ui/layout_two_columns`), so there is no page DOM here: the probe speaks the frames of `web/page_shim.js` over the team's live `/team_ws/ws`, the page's real leg, with its real grants (the dispatcher's `sidebar`, `tree`, `open`, `control`, and the chat verbs `history`, `send`, `stop`, `status`, `watch` on the workers and subagents).

Asserts, in order:

- the page is served, with the socket shim;
- one flat `items` list per group: each worker, then its `subagents` expander; workers have style `worker`, expanders style `subagent`; collapsed by default (no subagent listed);
- `expanded: ["w2"]` lists `sub_a` and `sub_b`;
- `open` for w1: `view chat`, `target w1`, `fullscreen`; a message sent to w1 reaches the model once (holding the unique token), and only w1's history holds the token (not w2, `sub_a`, `sub_b` or the lead);
- `open` for `sub_a`, then a second token: one more model request, and only `sub_a`'s history holds it.

The team serves the page with the `webui` word (`stock.serve_webui`). `sub_a` and `sub_b` are not members of the group, so their records (the test's `fleet/` folder) carry the chat grants for them.

Removed with the old page (it drew a sidebar and a pane): the checks that read the drawn rows (classes `worker`/`expander`/`sub`, `aria-expanded`, `subagents (0)`/`(2)` labels), the clicks on an expander, a row and Back, the pane, composer placeholder and Back button, the frames sent by the page's own handlers, and the remembered key `fantastic.team.subagents.w2`.
