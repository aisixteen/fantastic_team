# Overview: the team web UI

## Purpose and audience
The team web UI is the one page an operator opens to see the team of AI workers they run: which workers are busy, idle or stalled, what each one is on, and where to talk to one. The same page is served in a browser and inside the desktop app.

## Data shown
- `sidebar {expanded}` (dispatcher): one flat list per group. Each worker row shows a busy or idle dot, its id, its state and its current task, e.g. `api-worker`, `busy`, `aisixteen/fantastic_team#131`.
- `tree {node}` (dispatcher): the nested tree of team, group and worker, each row with its state, task, time and cost.
- `cells {worker}` (worker view): the generated cells of one worker's view.
- Every screen shows text from the kernel as plain text. No screen shows a secret value, only that a connection is set or missing.

## States
- loading: the page shows the layout with the title `Team` and no rows.
- empty: a team with no workers shows the title and the text `no workers yet`.
- error: a single line in the error colour under the title, e.g. `no connection to the team: fantastic is not loaded`.
- many items: a group with twelve workers lists all twelve, sorted by id.
- a stalled row: its state reads `stalled 45 min` in the stalled colour.

## Actions
- Tapping a worker row or a subagent row opens its full-screen chat. Back returns to the list with its scroll position and expanded workers kept.
- Tapping the subagents line under a worker expands or collapses that worker's subagents.
- No screen assigns work. The only write is `control {node, action, confirm}` for stop, and stop asks for a confirm first.

## Shared rules
- Light and dark themes both work: text and state colours keep their contrast in each.
- Phone width (up to 600 px) makes the list and the pane two panels side by side that snap into view: opening a node shows the pane, Back shows the list; nothing swipes from one agent to the next. Desktop width puts the sidebar beside the pane; the pane fills the height and the chat scrolls inside it.
- A chat shows an embedded agent as a card where the embed row sits, and a Canvas button swaps the chat for the agent's canvas when one exists. The tree folds per subtree and as a column.
- The same page in a browser and in the app: one set of files, no separate app screens.
- A brief shows data and sends verbs. It holds no logic beyond that: no sorting rules, no timers and no computed totals on the page.
- No auth, accounts, billing or SaaS chrome. No provider or model names in any layout.
- Lists are sorted by id. Sample names are plausible: `api-worker`, `docs-worker`, `ci-fixer`.
