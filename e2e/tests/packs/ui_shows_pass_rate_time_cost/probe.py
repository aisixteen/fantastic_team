"""The team page shows a pack's pass rate, time and cost: for the Eval button of a pack card and for an
install. The page's own script (`page.js`) runs on a stub DOM, bridged to the live /team_ws/ws of a team
that serves the page with the `webui` word, so the verbs, the leg's grants, the evals and the scripted
lead are all real."""

import os
import re

from helpers import packs_evals as P, packs_pub as PP, run_flow, stock as S, team as team_mod
from helpers.page import Page, PageError, node_ok
from helpers.verdict import Verdict

NAME = "packs/ui_shows_pass_rate_time_cost"
URL = "https://registry.example.invalid/p-1.0.0.json"


def run(ctx):
    v = Verdict(NAME)
    if not node_ok():
        print(f"SKIP {NAME}: needs node >= 22 (global WebSocket) to drive the page's script against the live socket")
        return 0
    run_flow.boot(ctx, packs=True)
    team = ctx.team
    pack = P.make_pack(team, "p", {"a.yaml": P.PASS_EVAL})
    # list the pack, so the page has a card to select (the Eval button is in its detail)
    PP.write_seed(team)
    pub = PP.publish(team, pack, URL, [])
    PP.trust(team, pub["published"]["signer_key"])
    port = team_mod.free_port()
    S.serve_webui(team, port)  # the binary makes the page and the leg's grants on the pack agents

    page = Page(port, os.path.join(ctx.run_dir, "page_driver.err"))
    try:
        page.cmd(cmd="load")
        cards = page.cmd(cmd="text", id="packs-cards")["text"]
        v.check("the packs section draws the listed pack as a card", "p 1.0.0" in cards, cards)

        # the Eval button: select the card, press Eval in its detail; the page asks for the pack and the worker
        page.cmd(cmd="click_in", id="packs-cards", sel="[data-pack]")
        detail = page.cmd(cmd="text", id="packs-detail")["text"]
        v.check("selecting the card shows its detail with an Eval button", "Eval" in detail, detail)
        page.cmd(cmd="prompts", answers=[pack, ctx.lead])
        page.cmd(cmd="click_in", id="packs-detail", sel='[data-act="eval"]', quiet_ms=1500)
        note = page.cmd(cmd="text", id="packs-note")["text"]
        v.check("the Eval button shows the reply with the pass rate, the time and the cost",
                '"pass_rate":1' in note and '"total_time_s":' in note and '"total_cost":' in note, note)

        # the install form with a worker: evals run first and the result shows the three lines
        for field, value in (("pack-pack", pack), ("pack-into", "templates"), ("pack-worker", ctx.lead)):
            page.cmd(cmd="set", id=field, value=value)
        page.cmd(cmd="click_id", id="pack-install", quiet_ms=1500)
        text = page.cmd(cmd="text", id="pack-result")["text"]
        shown = {"Pass rate: 100%": "Pass rate: 100%" in text,
                 "Time: <n> s": re.search(r"Time: \d+(\.\d+) s", text) is not None,
                 "Cost:": "Cost: " in text}
        v.check("the install shows the pass rate, the time and the cost", all(shown.values()), f"{shown} in {text!r}")
        v.check("the install did happen (the template is on disk)",
                os.path.exists(os.path.join(team.dir, "templates", "p-1.0.0", "manifest.yaml")))
    except PageError as e:
        v.check("every step of the page can be done", False, str(e))
    finally:
        page.close()
    code = team.stop()
    v.check("the team stopped on SIGTERM", code in (0, -15), f"exit {code}")
    return v.finish()
