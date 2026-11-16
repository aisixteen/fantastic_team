"""The tool agents' views, used (#351): the cadencer, the GitHub connector and the bundles registry, each
opened from the tree, show their own agent's real data, and their controls act on that agent. Function
only."""

from helpers import team_ui as D
from helpers.verdict import Verdict

NAME = "ui/tool_views_real_data"
CARDS, SIZE = True, (1440, 900)


def seed(ctx, v):
    """Two connector calls, so its log has rows: one allowed repo, one outside the allowlist."""
    D.op(ctx.team, "github", {"type": "issues", "repo": "acme/widgets"})
    D.op(ctx.team, "github", {"type": "issues", "repo": "other-org/tools"})
    calls = D.op(ctx.team, "github", {"type": "reflect"}).get("recent_calls") or []
    v.check("the connector logged the two calls made before the page opened", len(calls) >= 2, str(calls)[:200])


def run(ctx):
    v = Verdict(NAME)
    pw = D.playwright(v)
    if pw is None:
        return v.finish()
    port = D.start(ctx, v, cards=CARDS)
    if port is None:
        return v.finish()
    seed(ctx, v)
    try:
        with D.Ui(pw, port, *SIZE) as ui:
            if ui.tree_ready(v):
                clicks(ctx, v, ui)
            ui.no_errors(v)
    except Exception as e:  # a missing browser or a crashed page is a failure, never a skip
        v.check("the browser ran every step", False, f"{type(e).__name__}: {e}")
    D.stop(ctx, v)
    return v.finish()


def settings(ctx) -> dict:
    return D.op(ctx.team, "cadencer", {"type": "status"}).get("settings") or {}


def clicks(ctx, v, ui):
    right, team = ui.right, ctx.team

    # ---- the cadencer ----
    v.check("a click on the cadencer's row opens it", ui.open("cadencer"), ui.right_now())
    ok, got = ui.waits(right.locator("#state"), "on", exact=True)
    if v.check("the right cell is the cadencer's view: its switch reads on, as its record says", ok,
               f"{got!r}; {ui.right_now()}"):
        num = lambda label: right.locator(f'input[aria-label="{label}"]').first.input_value(timeout=3000)
        try:
            shown = [num("cadence"), num("stall after")]
        except Exception as e:
            shown = [type(e).__name__]
        v.check("it shows the record's cadence (60 s) and stall threshold (300 s)", shown == ["60", "300"], str(shown))
        sw = {t: ui.attr(right.locator('button[role=switch]').filter(has_text=t), "aria-checked") for t in ("Digests on", "Quiet")}
        v.check("its two switches read the record: digests on, quiet off", sw == {"Digests on": "true", "Quiet": "false"}, str(sw))
        off = right.locator('button[aria-label^="switch digests on for "]')
        v.check("it lists the lead whose digests the record switches off (m_ads)", ui.texts(off) == ["m_ads ×"], str(ui.texts(off)))
        st = D.op(team, "cadencer", {"type": "status"})
        figures = ui.texts(right.locator(".kpis .kpi b"))
        v.check("its figures are the cadencer's own counts (ticks, digests sent, delivery errors)",
                len(figures) == 3 and figures[1:] == [str(st.get("digests_sent", 0)), str(st.get("delivery_errors", 0))]
                and figures[0].isdigit() and int(figures[0]) <= int(st.get("ticks", 0)), f"shown {figures} status {str(st)[:200]}")
        ui.click(right.locator('button[role=switch]').filter(has_text="Quiet"))
        ok, got = ui.settle(lambda: settings(ctx).get("quiet"), True)
        v.check("a click on 'Quiet' sets it on the cadencer", ok, str(got))
        ok, got = ui.settle(lambda: ui.attr(right.locator('button[role=switch]').filter(has_text="Quiet"), "aria-checked"), "true")
        v.check("and the switch then reads on", ok, str(got))
        ui.click(off)
        ok, got = ui.settle(lambda: settings(ctx).get("off") or [], [])
        v.check("a click on 'm_ads ×' switches m_ads's digests back on in the cadencer", ok, str(got))
        try:
            box = right.locator('input[aria-label="cadence"]').first
            box.fill("45", timeout=4000)
            box.press("Tab", timeout=4000)
        except Exception:
            pass
        ok, got = ui.settle(lambda: settings(ctx).get("cadence"), 45)
        v.check("a cadence typed in the view (45) is set on the cadencer", ok, str(got))

    # ---- the GitHub connector ----
    v.check("a click on the GitHub connector's row opens it", ui.open("github"), ui.right_now())
    truth = D.op(team, "github", {"type": "reflect"})
    mode = (truth.get("auth") or {}).get("mode")
    ok, got = ui.waits(right.locator("#auth"), f"auth: {mode}", exact=True)
    if v.check(f"the right cell is the connector's view: its auth mode is the connector's ({mode})", ok,
               f"{got!r}; {ui.right_now()}"):
        dds = right.locator("dl.kv dd")
        v.check("it lists the record's repos (acme/widgets)", ui.texts(dds.nth(0).locator(".pill")) == ["acme/widgets"],
                str(ui.texts(dds.nth(0))))
        v.check("it shows the connector's api url (this run's forge)", ui.texts(dds.nth(1)) == [ctx.vars["forge_url"]],
                f"{ui.texts(dds.nth(1))} want {ctx.vars['forge_url']}")
        want = [[c.get("verb"), c.get("target") or "-", c.get("result")] for c in reversed(truth.get("recent_calls") or [])]
        rows = [ui.texts(r.locator("span")) for r in right.locator(".tbl .tr:not(.th)").all()]
        v.check("its recent calls are the connector's log, newest first", bool(want) and rows == want, f"shown {rows} log {want}")
        v.check("the call outside the allowlist shows as refused",
                any(r[0] == "issues" and "other-org/tools" in r[1] and r[2] == "refused" for r in rows), str(rows))
        try:
            right.locator('input[aria-label="repo"]').fill("acme/widgets", timeout=4000)
            right.locator('input[aria-label="pull request number"]').fill("7", timeout=4000)
            right.get_by_role("button", name="Look up").click(timeout=4000)
        except Exception:
            pass
        pr = D.op(team, "github", {"type": "pr_status", "repo": "acme/widgets", "number": 7})
        ok, got = ui.waits(right.locator(".sec").filter(has_text="Pull request").locator("dl.kv dd").first, lambda t: t.startswith(str(pr.get("state"))), timeout=20)
        v.check(f"a pull request looked up in the view shows the state the connector reports ({pr.get('state')})",
                ok and bool(pr.get("state")), f"shown {got!r} connector {str(pr)[:200]}")

    # ---- the bundles registry ----
    v.check("a click on the bundles registry's row opens it", ui.open("bundles"), ui.right_now())
    listed = D.op(team, "bundles", {"type": "bundles"}).get("bundles") or []
    names = [b.get("name") for b in listed]
    ok, _ = ui.waits(right.locator('button[aria-label^="kind "]'))
    kinds = right.locator('button[aria-label^="kind "]')
    if v.check("the right cell is the registry's view: its kinds are the bundles the registry lists",
               ok and bool(names) and ui.texts(kinds.locator("b")) == names, f"shown {ui.texts(kinds.locator('b'))} registry {names}; {ui.right_now()}"):
        running = D.op(team, "bundles", {"type": "get_count"}).get("running")
        v.check("its count is the registry's running agents", ui.texts(right.locator("#run")) == [f"{running} running"],
                f"{ui.texts(right.locator('#run'))} registry {running}")
        purposes = [ui.texts(k.locator("span"))[1] for k in kinds.all()]
        v.check("each kind shows its card's purpose", purposes == [str(b.get("purpose") or "") for b in listed], str(purposes)[:300])
        pick = "worker" if "worker" in names else names[-1]
        card = D.op(team, "bundles", {"type": "bundle", "name": pick})
        ui.click(right.locator(f'button[aria-label="kind {pick}"]'))
        ok, got = ui.waits(right.locator(".code pre"), lambda t: t[:60] == " ".join(str(card.get("directive") or "").split())[:60] and t != "", timeout=20)
        v.check(f"a click on the kind '{pick}' shows that card's directive", ok, got[:120])
        heads = ui.texts(right.locator(".sec h2"))
        v.check(f"the detail is headed by the kind picked ({pick})", pick in heads, str(heads))
