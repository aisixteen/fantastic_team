# ui/sidebar_renders (free)

The team page's left cell is the BASE sidebar view (fantastic#608), not a column of the team's own: the page asks the `sidebar` agent for its `reflect`, runs the view in a sandboxed iframe and relays what it sends over the page's leg (`/team_ws/ws`), with the leg's read grants for the view (`sidebar`, the root, `files`).

Headless Chromium (Playwright) opens `/team_page/file/index.html` in the team's real binary (`webui --port N`, scripted model, no key) and asserts, reading the view's own document (the iframe is null-origin; Playwright's frame locator reaches it):

- the view is drawn in `#body-left`: its Kernels rail, the Agents nav and the Files switch (off);
- agent rows are drawn: the lead (under `user`) and its child `child_one`;
- no file row while the switch is off; the switch on shows the project's files (`notes.txt`, the folder `docs`) as rows in the tree;
- a click on the lead's row opens it: the right cell carries `data-open="lead"`;
- no console error and no page error.

The test's `fleet/` lays what the product's blueprint seeds for this (`sidebar`, the leg's grants, the callee ends on the root and on `files`), since the e2e fleets are record files and the seed does not run here. Without Playwright the probe fails with a message (it never skips).
