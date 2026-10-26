# ui/worker_view_opens

Free tier (scripted model, no key). Two workers `w1` and `w2` in the group `workers`, the binary started with `--port`.

Checks:
- the worker view (`worker_view` verbs): bound to one worker, in the `mobile` layout; the default cells are task, files, output, preview, then the chat cell, which is last, locked, and targets that worker only; `open` is idempotent; `next`/`prev` cycle within the group and wrap; `place` adds above the chat and refuses a chat cell.
- the web UI of the binary (`--port`) is served, and its socket (`canvas_ws`) may not call `worker_view` or chat with the lead; nothing reaches the model through it.

Not built (printed as `[skipped]`, never a pass):
- The page opening a worker (dispatcher `open`, a full-screen chat bound to that worker). The binary's `--port` serves fantastic's canvas. The dispatcher view is not placed on it on a fresh start (`canvas_page` does not exist yet when the tree is applied: "dispatcher not placed on `canvas_page`"), and `canvas_ws` may not call the dispatcher. The dispatcher page with its own socket (`team_page`, `team_ws`) is served only by the in-process app (`TeamHandle`), not by the binary.
- No horizontal scroll at 360x740 and a desktop width. No page serves the worker view's cells or `worker_view.css` (the web UI opens the chat only), and the harness has no headless browser client.
- Default top cells and next/prev "through the web UI": the page cannot reach `worker_view`; they are checked on the verb leg.
