# packs/ui_shows_pass_rate_time_cost (free)

Issue #41: the web UI shows a pack's pass rate, time and cost.

The packs panel (`crates/team/web/packs.html`) runs in `e2e/helpers/panel_dom.js`: node, a stub DOM, no dependencies, and a `fantastic.send` that runs `fantastic_team send` against the LIVE team (so the verbs, the evals and the scripted lead are real). The probe packs a worker with one eval, then presses the panel's Eval button and its install (Preview) button with the worker field set, and asserts that the result text of each holds `pass rate 100%`, a `time <n>s` and the `cost`, and that the install happened.

Needs `node`; without it the test prints `SKIP: node not found` and passes. No browser, WebKit or display is needed.
