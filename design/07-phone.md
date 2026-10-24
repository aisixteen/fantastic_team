# Phone: the layout of the screens on a narrow display

## Purpose and audience
An operator checks the team from a phone, standing up. This brief sets how every screen above reflows at phone width, so the same content reads well in one column.

## Data shown
- The same data as the screens it applies to: the dispatcher rows, a group's workers, a worker's cells, the asks, the connections and the settings.
- Example: on a 390 px wide display the dispatcher shows the group `backend` as a title and three rows in one column.

## States
- loading, empty, error, many items and a stalled row look the same as on desktop, stacked in one column.
- A long task name or file path wraps inside its row and never widens the page.

## Actions
- Tapping a row works the same as on desktop. The chat sits locked at the bottom of a worker's canvas.
- Back returns to the list in the same place.

## Shared rules
- Phone width is up to 430 px. Below it the sidebar stacks above the pane; the worker view is one vertical canvas.
- Each side has a 16 px gutter. No screen scrolls sideways.
- Light and dark themes both work at phone width.
- The same page in a browser and in the app: one set of files for both sizes.
- The page shows data and sends verbs only.
