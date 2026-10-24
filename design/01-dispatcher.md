# Dispatcher: the main view

## Purpose and audience
The dispatcher is the first screen of the team. An operator looks here to see every group and worker at a glance, and to open any of them. It is the app's and the web UI's main view.

## Data shown
- `sidebar {expanded}` (dispatcher agent): answers `groups`, each with an `id` and its `items`, plus `ungrouped` for workers in no group, and `team` for the title.
  - Example: group `backend` with the items `api-worker` (busy, `aisixteen/fantastic_team#131`), `db-worker` (idle), and the expander `subagents (2)` under `api-worker`.
  - Example: group `frontend` with the items `ui-worker` (stalled 35 min) and `docs-worker` (done).
- Under each group title, one plain line `busy 2 · idle 1 · stalled 0`, only for a group with no AI driver. A group with a driver shows its driver's summary instead.
- `tree {node}` (dispatcher agent): the same workers as a tree from team to group to worker, each row with state, task, time and cost.
- Subagent rows, shown only under an expanded worker: id, tool and status, e.g. `search-agent`, `grep`, `running`.

## States
- loading: the title reads `Team`; the sidebar is empty.
- empty: no groups and no workers; the sidebar says `no workers yet`.
- error: one line under the title with the message from the reply, e.g. `no connection to the team: fantastic is not loaded`. The sidebar keeps its last list.
- many items: a group of twelve workers lists all twelve sorted by id, with the expander rows between them.
- a stalled row: its state text is in the stalled colour, e.g. `stalled 45 min`.
- a failed row: its state text is in the failed colour, e.g. `failed`.
- a done row: its state text is in the done colour, e.g. `done`.

## Actions
- Tapping a worker row sends `open {node}` and shows that worker's full-screen chat. Back returns to the sidebar.
- Tapping a subagent row sends `open {node}` for that subagent and shows its chat the same way.
- Tapping the `subagents (n)` line sends `sidebar {expanded}` with that worker added or removed, and remembers the choice in this browser.
- Back, in the header, returns from a chat to the sidebar and restores the scroll position.

## Shared rules
- Light and dark themes both work: group titles, state text and the error line keep their contrast in each.
- Phone width up to 430 px stacks the sidebar above the pane, with a 16 px gutter and no horizontal scroll. Desktop width puts the sidebar and pane side by side.
- The same page in a browser and in the app, served from the same file.
- The page shows what the dispatcher answers and sends verbs. It assigns no work.
