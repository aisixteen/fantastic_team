# packs/web_browser

Issue #42. The packs section of the team page (`crates/team/web/team.html`) and the verbs it sends. The old `packs_view` agent and `packs.html` are gone (#212: the team page serves the Pack install panel and the packs browser); the behaviour stayed, so this test reads the page where it lives now (#225).

Real rendering is NOT exercised: there is no browser, display or WebKitGTK in this tier, and the page is JavaScript. The test checks the contract instead:

- the team page as the binary serves it (`webui --port N`, `stock.serve_webui`): its script names `browser`, `show`, `fetch` for `pack_listing` and `install`, `eval` for `pack`; its socket leg `team_ws` may call `pack` with `install` and `eval` and `pack_listing` with `browser`, `show` and `fetch`, and no more; the script makes no request of its own (no `fetch(`, no `XMLHttpRequest`);
- the exact payloads: `list` (with and without a query) and `show` (ports, grants, price, install decision), the install preview (`parent_id`, no `approve`: the permission preview under `approval`, no agent created) then the same payload with `approve` (the blueprint is imported and the new agent exists), and `eval` on the lead (the scripted model answers `ok`);
- the browser view-model: `browser` shows updates and their permission diff only with `opt_in`, and says `turn on updates to see changes` without it.

Free tier: scripted model, no key, no network. `ui_shows_pass_rate_time_cost` drives the page's own buttons.

One fact the test works around, about the harness and not the page: the operator grants `pack` the `import_subtree` verb on the seeded `user` agent (the tree's own grant is written only when `user` is created by the plan; the first start seeds it, see e2e/README.md). `eval` reads a model agent's answer from `final` (the lead has no `text`).
