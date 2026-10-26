# packs/web_browser

Issue #42. The pack browser panel (`crates/team/web/packs.html`) and the verbs it sends.

Real rendering is NOT exercised: there is no browser, display or WebKitGTK in this tier, and the page is JavaScript. The test checks the contract instead:

- the panel as the team serves it: the `packs_view` agent the first start places on the canvas (the harness adds the `canvas_page` the app would make; this binary has no `html.tools` bundle, so the agent is read from its persisted record), its page names `pack_listing`, `install`, `eval`, and its `can_call` is the two pack agents only;
- the exact payloads the page sends: `list` (with and without a query), `show` (ports, grants, price, install decision), the install preview (`parent_id`, no `approve`: the permission preview under `approval`, no agent created) then the same payload with `approve` (the blueprint is imported and the new agent exists), and `eval` on the lead (the scripted model answers `ok`);
- the browser view-model: `browser` shows updates and their permission diff only with `opt_in`, and says `turn on updates to see changes` without it.

Free tier: scripted model, no key, no network.

Two facts the test works around, both about the harness and not the panel: the harness adds the `canvas_page` the app makes, and the operator grants `pack` the `import_subtree` verb on the seeded `user` agent (the tree's own grant is written only when `user` is created by the plan; the first start seeds it, see e2e/README.md). `eval` reads a model agent's answer from `final` (the lead has no `text`).
