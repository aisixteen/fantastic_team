# Worker canvas: one worker's vertical view

## Purpose and audience
Opening a worker shows its own vertical canvas, like a phone screen: what it is doing, the files it touches, its output and a preview, with its chat locked at the bottom. The operator uses it to follow one worker closely and to message it like a colleague.

## Data shown
- `open {worker}` (worker view) answers the worker's view: its layout `mobile` and its cells.
- `cells {worker}` (worker view) answers the generated cells, top to bottom:
  - `task`: the current task, e.g. `aisixteen/fantastic_team#131`, with its state `busy` and its time.
  - `files`: the files the worker touches, sorted, e.g. `crates/team/src/worker_view.rs`, `crates/team/web/worker_view.css`.
  - `output`: the last lines of the worker's build or test output, e.g. `test result: ok. 42 passed`.
  - `preview`: a preview of the result when the worker has one.
  - `note`: a short note the worker or the board placed there.
- `chat` (the last cell): the worker's chat, targeting that worker id only.
- Example: worker `api-worker` with a task card, two files, one output line and a chat.

## States
- loading: the task card placeholder and the chat box, both with no text yet.
- empty: a worker with no task reads `no task` in its task card; the chat still works.
- error: the chat cell shows the reply's message, e.g. `worker api-worker is not reachable`.
- many items: a worker with eight files lists all eight, sorted, in the files cell, and the cell scrolls inside the top part.
- a stalled row: the task card reads `stalled 40 min` in the stalled colour.

## Actions
- The chat at the bottom sends a message to this worker only. It is always visible; the top part scrolls.
- Back returns to the dispatcher.
- No cell assigns work or edits a file.

## Shared rules
- Light and dark themes both work. Cells use the mobile theme's colours.
- Phone width: one column, the cells stacked, the chat locked at the bottom, no horizontal scroll. Long file paths wrap anywhere so they never widen the page.
- The same page in a browser and in the app.
- The page shows data and sends verbs only.
