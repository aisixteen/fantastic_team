# Connections: which names the team has

## Purpose and audience
This panel shows which connections a team declares and whether each is set. The operator checks it before a run, to see what is missing. It never shows a value.

## Data shown
- The connections of a team (dispatcher agent, from the names the team declares): one row per name, with `set` or `missing`.
- Example rows, sorted by name: `GITHUB_TOKEN` set; `RELAY_URL` missing.
- Only the name and the presence. A connection's value never appears on the page.

## States
- loading: the header reads `Connections` with no rows.
- empty: a team with no declared connections reads `no connections declared`.
- error: one line under the title with the reply's message.
- many items: a team with nine connections lists all nine, sorted by name.
- a missing row: the status reads `missing` in the failed colour.

## Actions
- None on the page. A missing connection is set outside the page.

## Shared rules
- Light and dark themes both work; `set` and `missing` differ by the word, not by colour alone.
- Phone width: one column, rows wrap, no horizontal scroll, 16 px gutter.
- The same page in a browser and in the app.
- The page shows presence only and sends no verbs.
