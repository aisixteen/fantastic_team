# ui/sidebar_opens_agent_full_screen (free)

The dispatcher page, driven end to end against the real binary: one group `fantastic` (no driver, no board) with two workers, `w1` and `w2`, and two subagents of `w2` (`sub_a`, `sub_b`: kernel agents whose parent is `w2`), all on the scripted model. The model's script is two `done` turns, one per message sent.

There is no browser: `helpers/page_driver.js` (node >= 22, a global WebSocket) fetches the served page, runs the page's own script on a small stub DOM, and bridges its `fantastic.send` and `fantastic.watch` to the team's live `/team_ws/ws` with the frames of `web/page_shim.js`. Clicks go through the page's own handlers. The page's socket leg has only the grants of the app (the dispatcher's `sidebar`, `tree`, `open`, `control`, and the chat verbs `history`, `send`, `stop`, `status`, `watch` on the workers and subagents). Without node >= 22 the test prints `SKIP` and passes.

Asserts, in order:

- the page is served and holds the sidebar script and the socket shim;
- one flat `items` list per group: each worker, then its `subagents` expander; workers have style `worker`, expanders style `subagent`; drawn, the worker and expander rows are siblings in the sidebar nav with classes `worker` and `expander`;
- collapsed by default: the first `sidebar` request carries `expanded: []`, both expanders have `aria-expanded` false and read `subagents (0)` and `subagents (2)`, no `sub` row is drawn;
- a click on w2's expander asks for `expanded: ["w2"]`, draws `sub_a` and `sub_b` (class `sub`) after w2, and leaves w1 collapsed;
- a click on w1: `open` answers `view chat`, `target w1`, `fullscreen`; the pane is shown, the sidebar hidden, the title is `w1`, the composer says `Message w1`;
- a message typed there reaches the model once (exactly one request, holding the unique token), and only w1's history holds the token (not w2, `sub_a`, `sub_b` or the lead);
- Back, then a click on `sub_a` (the expansion is kept): the same for the subagent, every `history`, `send` and `watch` frame sent meanwhile names `sub_a`, a second token costs exactly one more model request and only `sub_a`'s history holds it;
- Back again: the pane is hidden, the sidebar shown, Back hidden, the reload asks for `expanded: ["w2"]`, w2's subagents are still drawn, w1 is collapsed, the remembered key `fantastic.team.subagents.w2` is `1` (none for w1) and the scroll was restored.

What the harness works around: the workers and subagents are made after boot, so the probe writes, with operator verbs, the chat grants that `tree::grant_chat_to` writes at start for the workers a team.yaml names (`grant_chat`). The page, the `/team_ws/ws` leg, the dispatcher's grants and the model calls are the binary's own: `--port` serves them through the same code as the app (`app_control::persist_web`, which runs `prepare_web`).

Mutations (each makes named checks fail, and each was run once):

- `toggleWorker` also expands w1: the expanded-set checks fail (the click's `expanded: [w2]` request, "w1's expander is still collapsed", and the two Back checks);
- `openNode` shows the chat for `lead` instead of the clicked node: the title and composer, the model-request count, the history and frame checks fail;
- `back()` no longer restores the expanded set: "Back from w1's chat shows the sidebar with w2 still expanded" fails, and the later steps cannot be done;
- the `subagents` expander item gets style `worker` in `dispatcher.rs` `worker_items`: "workers have style worker; expanders have style subagent" fails.
