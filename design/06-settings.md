# Settings: the team's read-only values

## Purpose and audience
This screen shows the values a team is running with, so an operator can see the stall limit, the board and the groups without opening the configuration file. It changes nothing.

## Data shown
- `team` (dispatcher header): the team's name, e.g. `fantastic_team`.
- `stall minutes`: the limit after which a worker with no progress counts as stalled, e.g. `30`.
- `board`: the name of the board the team reads, e.g. `board`.
- `groups`: the group ids in the order the team declares them, e.g. `backend`, `frontend`.
- Each value is text; none is editable on the page.

## States
- loading: the labels with no values.
- empty: a team with no groups reads `no groups declared`.
- error: one line under the title with the reply's message.
- many items: a team with eight groups lists all eight, sorted by id.

## Actions
- None. Changing a value means editing the team's configuration file and starting the team again.

## Shared rules
- Light and dark themes both work, labels and values keep their contrast in each.
- Phone width: one column, rows wrap, no horizontal scroll, 16 px gutter.
- The same page in a browser and in the app.
- The page shows values only and sends no verbs.
