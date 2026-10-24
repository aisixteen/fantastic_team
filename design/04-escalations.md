# Escalations: what is waiting for the person

## Purpose and audience
This screen lists what a group's driver or a worker has asked the person to decide. The operator reads the asks in order and opens the worker that raised one.

## Data shown
- The board's asks (board agent): each one a record with `from`, `to`, its text, and its answer when it has one, oldest first. Example: `api-worker` asks `maintainer`: `Shall the retry limit be 3?`, answered `yes`.
- Each ask names the task it belongs to, e.g. `aisixteen/fantastic_team#95`.
- A driver's escalation reads the same way, under the group title.
- Counts for the team header: `open asks 2 · answered 5`.

## States
- loading: the header reads `open asks` with no count.
- empty: the list reads `nothing is waiting for you`.
- error: one line under the title with the reply's message.
- many items: twelve asks list all twelve, oldest first, with the open ones first.
- a stalled row: an ask unanswered for more than the stall limit reads `waiting 45 min` in the stalled colour.

## Actions
- Tapping an ask opens the chat of the worker that raised it.
- No answer action exists in the page yet; the list only shows.

## Shared rules
- Light and dark themes both work; the open and answered rows differ by text, not by colour alone.
- Phone width: one column, rows wrap, no horizontal scroll, 16 px gutter.
- The same page in a browser and in the app.
- The page shows data and sends verbs only.
