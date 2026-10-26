# ui/worker_view_opens

Free tier (scripted model, no key). Two workers `w1` and `w2` in the group `workers`, the binary started with `--port`.

Checks:
- the worker view (`worker_view` verbs): bound to one worker, in the `mobile` layout; the default cells are task, files, output, preview, then the chat cell, which is last, locked, and targets that worker only; `open` is idempotent; `next`/`prev` cycle within the group and wrap; `place` adds above the chat and refuses a chat cell.
- the web UI of the binary (`--port`) is served, its socket (`canvas_ws`) may not call `worker_view`, the dispatcher or the lead or w1; nothing reaches the model through it. `/team_page` is a 404 and `/team_ws/ws` reaches `canvas_ws`, so the binary has no dispatcher page (shown, not assumed).
- chat reaches only the bound worker: a `send` to w1 reaches the scripted model and shows in the log under w1 only (w2 and the lead handle nothing).

Not built (printed as `[skipped]`, never a pass):
- The page opening a worker (dispatcher `open`, a full-screen chat bound to that worker). The dispatcher page with its socket (`team_page`, `team_ws`, and `grant_chat_to` for the group members) is made only by the in-process app (`TeamHandle`), not by the binary's `--port`, which serves fantastic's canvas. The maintainer should know: the binary has no page that opens a worker. A probe cannot start `TeamHandle`; its own unit tests cover it.
- No horizontal scroll at 360x740 and a desktop width. No page serves the worker view's cells or `worker_view.css` (the web UI opens the chat only), and the harness has no headless browser client.
- Default top cells and next/prev "through the web UI": the page cannot reach `worker_view`; they are checked on the verb leg.
