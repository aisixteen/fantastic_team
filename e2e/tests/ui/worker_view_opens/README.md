# ui/worker_view_opens

Free tier (scripted model, no key). Two workers `w1` and `w2` in the group `workers`, the binary started with `--port`.

Checks:
- the worker view (`worker_view` verbs): bound to one worker, in the `mobile` layout; the default cells are task, files, output, preview, then the chat cell, which is last, locked, and targets that worker only; `open` is idempotent; `next`/`prev` cycle within the group and wrap; `place` adds above the chat and refuses a chat cell.
- the web UI of the binary (`--port`) is served, and so is the dispatcher page (`/team_page/file/index.html`, the same code as the app). Its socket `/team_ws/ws` answers dispatcher `open` for w1 with `view chat`, `target w1`, `fullscreen`, and may not call `worker_view` or the lead; nothing reaches the model through the refused calls.
- chat reaches only the bound worker: a `send` to w1 over `/team_ws/ws` (the probe grants the runtime-made workers to the leg, as `grant_chat_to` does at start) reaches the scripted model and shows in the log under w1 only (w2 and the lead handle nothing).

Not built (printed as `[skipped]`, never a pass):
- No horizontal scroll at 360x740 and a desktop width: the harness has no headless browser client.
- Default top cells and next/prev "through the web UI": the page does not draw the worker view's cells; they are checked on the verb leg.
