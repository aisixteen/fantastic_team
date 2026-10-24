# Group canvas: one group of workers

## Purpose and audience
Opening a group shows the canvas of that group: a dispatcher bound to the group's own workers. An operator uses it to see how one group is moving, without the rest of the team.

## Data shown
- `open {node}` (dispatcher agent) for a group answers with the kind `group`, and points at a dispatcher bound to that group node.
- `tree {node}` (dispatcher agent) with the group id: the group title, then its workers in id order, e.g. `api-worker`, `db-worker`, `search-worker`, each with state, task, time and cost.
- The group's load line when the group has no AI driver: `busy 2 · idle 1 · stalled 1`. A group with a driver shows its driver's summary line in its place.
- Example group `backend` with three workers: `api-worker` busy on `aisixteen/fantastic_team#131` for 12 min, `db-worker` idle, `search-worker` stalled 40 min.

## States
- loading: the group title with the load line greyed out.
- empty: a group with no workers reads `this group has no workers yet`.
- error: one line under the title with the reply's message, e.g. `cannot open backend`.
- many items: a group of twelve workers lists all twelve sorted by id, one row each.
- a stalled row: its state reads `stalled 40 min` in the stalled colour.

## Actions
- Tapping a worker row opens that worker's full-screen chat, as on the dispatcher.
- Back returns to the dispatcher.
- A group does not assign work and has no edit action.

## Shared rules
- Light and dark themes both work, with the stalled and failed colours readable in each.
- Phone width up to 430 px stacks the rows in one column with a 16 px gutter and no horizontal scroll.
- The same page in a browser and in the app.
- The page shows data and sends verbs only. It holds no logic beyond that.
