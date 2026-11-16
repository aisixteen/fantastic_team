# ui/layout_two_columns (free)

The team page's layout in a real browser. The team binary serves the page on a port (the stock fleet with the `team_page` layer: the `web` host, `team_page` and its `team_ws` leg); headless Chromium, driven by Playwright (Python), opens `/team_page/file/index.html`. This test is about where the cells sit.

Needs Playwright: `python3 -m pip install --user playwright==1.55.0` (CI does it before the e2e run). Without it the probe fails with that message; it never skips.

Checks:
- 1440x900: `#col-left` and `#col-right` are both visible, share one top (within 2px), the left one ends before the right one begins, both are at least 400px tall;
- 390x844 (one panel at a time): the tree panel `#col-left` shows and the view panel `#col-right` is hidden, the shown panel is at least 340px wide; with the page set to the agent panel (`data-panel="agent"`) the view shows, the tree is hidden and the back button `#back` is visible; a click on it returns to the tree; the page does not scroll sideways (`scrollWidth <= 390`);
- no console error, and the team stops on SIGTERM.
