# packs/ui_shows_pass_rate_time_cost (free)

Issue #41: the web UI shows a pack's pass rate, time and cost.

The team page (`crates/team/web/team.html`) runs in `e2e/helpers/page_driver.js` (via `helpers/page.py`): node >= 22, a stub DOM and the page's own script `page.js`, bridged to the LIVE `/team_ws/ws` of a team the binary serves with `webui --port N` (`stock.serve_webui`), so the verbs, the socket leg's grants, the evals and the scripted lead are real. The old `packs.html` panel is gone (#212); its two buttons moved into the team page (#225):

- the Eval button of a listed pack's card (the probe packs a worker with one eval, publishes it so the page has a card, selects the card and presses Eval; the page asks for the pack folder and the worker): the page shows the reply as JSON in the packs note, and the test asserts it holds `pass_rate` 1, `total_time_s` and `total_cost`;
- the Pack install form with a worker set: the evals run first and the page writes `Pass rate: 100%`, `Time: <n> s` and `Cost: ...`; the install did happen (the template is on disk).

Needs node >= 22; without it the test prints `SKIP` and passes. No browser, WebKit or display is needed.
