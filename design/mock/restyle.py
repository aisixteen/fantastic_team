#!/usr/bin/env python3
"""Restyle the team mock sources to the base canvas cell look, black/white day/night, translucent panels.
Reads the untouched backup, writes the mock's src/. Rerunnable."""
import re, sys
from pathlib import Path

BACKUP = Path(sys.argv[1])
OUT = Path(sys.argv[2])

FONTS = ("[data-theme]{--sans:system-ui,-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;"
         "--mono:ui-monospace,SFMono-Regular,Menlo,monospace}")

DAY = ("[data-theme=\"light\"]{--bg:#EDEDED;"
       "--page:radial-gradient(circle at 1px 1px,rgb(0 0 0/.14) 1px,transparent 1.6px) 0 0/22px 22px,"
       "radial-gradient(70% 55% at 12% -5%,#fff 0,transparent 70%),"
       "radial-gradient(55% 60% at 105% 105%,rgb(0 0 0/.16) 0,transparent 70%),#EDEDED;"
       "--panel:rgb(255 255 255/.58);--panel-flat:rgb(255 255 255/.72);"
       "--side:rgb(0 0 0/.03);--surface:rgb(255 255 255/.72);--sunken:rgb(0 0 0/.06);--hover:rgb(0 0 0/.09);"
       "--line:rgb(0 0 0/.10);--line2:rgb(0 0 0/.18);"
       "--text:#0B0B0B;--text2:#454545;--text3:#6B6B6B;"
       "--accent:#111111;--accent-weak:rgb(0 0 0/.08);--accent-line:rgb(0 0 0/.30);--on-accent:#FFFFFF;"
       "--ok:#1B7446;--warn:#8F5200;--bad:#B3261E;"
       "--knock:#F8F8F8;--head:rgb(0 0 0/.70);--chip:rgb(0 0 0/.05);--chip-line:rgb(0 0 0/.06);"
       "--ext:rgb(0 0 0/.12);--ext-line:rgb(0 0 0/.40);"
       "--bubble:rgb(255 255 255/.60);--bubble-user:rgb(0 0 0/.10);--code:rgb(0 0 0/.045);--well:rgb(0 0 0/.06);--input:rgb(0 0 0/.06);"
       "--shadow-bezel:0 10px 30px rgb(0 0 0/.10),0 1px 2px rgb(0 0 0/.06);"
       "--shadow-bubble:inset 0 0 0 1px rgb(0 0 0/.07)}")

NIGHT = ("[data-theme=\"dark\"]{--bg:#050505;"
         "--page:radial-gradient(circle at 1px 1px,rgb(255 255 255/.14) 1px,transparent 1.6px) 0 0/22px 22px,"
         "radial-gradient(70% 55% at 12% -5%,rgb(255 255 255/.20) 0,transparent 70%),"
         "radial-gradient(55% 60% at 105% 105%,rgb(255 255 255/.10) 0,transparent 70%),#050505;"
         "--panel:rgb(20 20 20/.58);--panel-flat:rgb(20 20 20/.74);"
         "--side:rgb(255 255 255/.03);--surface:rgb(255 255 255/.07);--sunken:rgb(255 255 255/.07);--hover:rgb(255 255 255/.10);"
         "--line:rgb(255 255 255/.10);--line2:rgb(255 255 255/.18);"
         "--text:#F2F2F2;--text2:#B8B8B8;--text3:#8E8E8E;"
         "--accent:#F5F5F5;--accent-weak:rgb(255 255 255/.10);--accent-line:rgb(255 255 255/.32);--on-accent:#0B0B0B;"
         "--ok:#72D6A2;--warn:#F0B75A;--bad:#FF9288;"
         "--knock:#0E0E0E;--head:rgb(255 255 255/.70);--chip:rgb(255 255 255/.05);--chip-line:rgb(255 255 255/.06);"
         "--ext:rgb(255 255 255/.14);--ext-line:rgb(255 255 255/.42);"
         "--bubble:rgb(255 255 255/.07);--bubble-user:rgb(255 255 255/.16);--code:rgb(0 0 0/.35);--well:rgb(0 0 0/.38);--input:rgb(255 255 255/.06);"
         "--shadow-bezel:0 10px 30px rgb(0 0 0/.45),0 1px 2px rgb(0 0 0/.30);"
         "--shadow-bubble:inset 0 1px 0 rgb(255 255 255/.10)}")

# Status tints derive from the hue, as the canvas skin does (hue at 10-12 %, ring at 45 %).
MIX = ("[data-theme]{--ok-weak:color-mix(in srgb,var(--ok) 12%,transparent);"
       "--warn-weak:color-mix(in srgb,var(--warn) 12%,transparent);--warn-line:color-mix(in srgb,var(--warn) 45%,transparent);"
       "--bad-weak:color-mix(in srgb,var(--bad) 12%,transparent);--bad-line:color-mix(in srgb,var(--bad) 45%,transparent)}")

BEZEL = "padding: 0 6px 6px; border: 1px solid var(--line); border-radius: 16px; background: var(--panel); box-shadow: var(--shadow-bezel)"
HEAD = ("flex: none; display: flex; align-items: center; gap: 6px; height: 32px; padding: 0 6px; "
        "font: 600 11px/1 var(--mono); letter-spacing: .5px; text-transform: uppercase; color: var(--head)")
CHIP = ("flex: none; display: grid; place-items: center; min-width: 20px; height: 20px; padding: 0 5px; border: 1px solid var(--chip-line); "
        "border-radius: 4px; background: var(--chip); color: var(--head); font: 600 10px/1 var(--mono); letter-spacing: .5px; text-transform: uppercase")
EXT = CHIP.replace("var(--chip-line)", "var(--ext-line)").replace("background: var(--chip)", "background: var(--ext)").replace("color: var(--head)", "color: var(--text)")
BODY = "border-radius: 10px; overflow: hidden"

MAP = {3: 3, 4: 4, 5: 4, 6: 6, 7: 6, 8: 6, 9: 8, 10: 8, 11: 8, 12: 10, 13: 10, 14: 12, 16: 12}


def radii(style):
    """Canvas radius scale; a radius of half the height or more is a pill."""
    def one(m):
        r = int(m.group(1))
        hs = [int(h) for h in re.findall(r"(?<![-\w])(?:height|line-height): (\d+)px", style)]
        if hs and r * 2 >= min(hs) and r >= 7:
            return "border-radius: 999px"
        return "border-radius: %dpx" % MAP.get(r, r)
    return re.sub(r"border-radius: (\d+)px", one, style)


def sub1(text, old, new, count=1, name=""):
    n = text.count(old)
    if n != count:
        raise SystemExit("%s: expected %d of %r, found %d" % (name, count, old[:70], n))
    return text.replace(old, new)


def common(t, name):
    t = re.sub(r'<link rel="stylesheet" href="https://fonts\.googleapis[^\n]*\n', "", t)
    t = re.sub(r"\[data-theme\]\{--sans:[^\n]*", lambda m: FONTS, t)
    t = re.sub(r'\[data-theme="light"\]\{[^\n]*', lambda m: DAY, t)
    t = re.sub(r'\[data-theme="dark"\]\{[^\n]*', lambda m: NIGHT + "\n" + MIX, t)
    assert MIX in t and DAY in t, name
    t = t.replace(".field:focus-within{border-color:var(--accent) !important;box-shadow:0 0 0 3px var(--accent-weak)}",
                  ".field:focus-within{border-color:var(--accent-line) !important;box-shadow:inset 0 0 0 1px var(--accent-line) !important}")
    # radii, inside every style attribute
    t = re.sub(r'style="([^"]*)"', lambda m: 'style="%s"' % radii(m.group(1)), t)
    # badge text and knock-out rings: opaque, since the panels are see-through now
    t = t.replace("color: var(--surface)", "color: var(--knock)")
    t = re.sub(r"box-shadow: 0 0 0 (2|1\.5)px var\(--(surface|side|bg|warn-weak)\)", r"box-shadow: 0 0 0 \1px var(--knock)", t)
    t = t.replace("bfg: 'var(--surface)'", "bfg: 'var(--knock)'").replace("bbg: 'var(--surface)'", "bbg: 'var(--knock)'")
    t = t.replace("const ring = on ? 'var(--accent-weak)' : 'var(--side)';", "const ring = 'var(--knock)';")
    # role tiles: tint over whatever is behind
    t = re.sub(r"(\d+)%, var\(--surface\)\)", "20%, transparent)", t)
    # small caps labels in the cell head's voice
    t = t.replace("font-size: 11px; font-weight: 600; letter-spacing: 0.06em; text-transform: uppercase",
                  "font: 600 11px/1.4 var(--mono); letter-spacing: .5px; text-transform: uppercase")
    return t


def head(label, chips):
    return ('<div style="%s"><span style="flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis">%s</span>%s</div>'
            % (HEAD, label, chips))


def desktop(t, name, path, view):
    # page: gaps between cells, the background shows between and through them
    t = sub1(t, 'style="display: flex; flex-wrap: wrap; min-height: 900px; background: var(--bg); color',
             'style="display: flex; flex-wrap: wrap; gap: 12px; min-height: 900px; padding: 12px; background: var(--page); color', name=name)
    # the hide-the-tree control becomes a chip in the left cell's head
    m = re.search(r'<button type="button" class="hov" aria-label="Hide the tree"[^\n]*?(<svg[^\n]*?</svg>)</button>\n', t)
    assert m, name
    t = t.replace(m.group(0), "")
    svg = m.group(1).replace('width="16" height="16"', 'width="12" height="12"')
    hide = '<button type="button" class="hov" aria-label="Hide the tree" style="%s; padding: 0">%s</button>' % (CHIP, svg)
    left = ('<div style="flex: 1 1 376px; min-width: 0; position: sticky; top: 12px; align-self: flex-start; height: calc(100vh - 24px); '
            'display: flex; flex-direction: column; %s">\n%s\n<div style="flex: 1; min-height: 0; display: flex; %s">\n'
            % (BEZEL, head('team <span style="opacity: .55">/</span> launch-week', hide), BODY))
    t = sub1(t, '\n<sc-if value="{{rail}}" hint-placeholder-val="{{true}}">\n<nav aria-label="Kernels"',
             '\n' + left + '<sc-if value="{{rail}}" hint-placeholder-val="{{true}}">\n<nav aria-label="Kernels"', name=name)
    t = sub1(t, "padding: 12px 0; background: var(--bg); border-right: 1px solid var(--line)",
             "padding: 12px 0; background: transparent; border-right: 1px solid var(--line)", name=name)
    t = sub1(t, 'style="flex: 1 1 300px; min-width: 0; position: sticky; top: 0; height: 100vh; display: flex; flex-direction: column; gap: 14px; padding: 14px 10px 10px; background: var(--side); border-right: 1px solid var(--line)"',
             'style="flex: 1 1 0; min-width: 0; display: flex; flex-direction: column; gap: 14px; padding: 12px 8px 8px; background: transparent"', name=name)
    crumbs = path.split(" / ")
    label = "".join('<span style="opacity: .55">%s / </span>' % c for c in crumbs[:-1]) + crumbs[-1]
    right = ('</nav>\n</div>\n</div>\n\n<div style="flex: 999 1 560px; min-width: 0; display: flex; flex-direction: column; %s">\n%s\n'
             '<main style="flex: 1; min-width: 0; display: flex; flex-direction: column; background: transparent; %s">'
             % (BEZEL, head(label, '<span style="%s">%s</span>' % (EXT, view)), BODY))
    t = sub1(t, '</nav>\n\n<main style="flex: 999 1 560px; min-width: 0; display: flex; flex-direction: column; background: var(--surface)">', right, name=name)
    t = sub1(t, "</main>\n</div>\n</x-dc>", "</main>\n</div>\n</div>\n</x-dc>", name=name)
    return t


def chat(t, name, n_p, n_tool):
    # the message this agent was sent: a bubble on the right
    t, n = re.subn(r'<div style="display: flex; flex-direction: column; gap: 4px">\n(<div style="font: 11px/1\.4 var\(--mono\); color: var\(--text3\)">[^\n]*</div>)\n<div style="padding: (\d+px \d+px); border-radius: \d+px; background: var\(--sunken\)',
                   r'<div style="display: flex; flex-direction: column; align-items: flex-end; gap: 4px">\n\1\n<div style="max-width: 85%; padding: 8px 12px; border-radius: 12px; background: var(--bubble-user); box-shadow: var(--shadow-bubble)', t)
    assert n == 1, (name, "sender", n)
    # what the agent says: bubbles on the left
    t = sub1(t, '<p style="margin: 0">', '<p style="margin: 0; align-self: flex-start; max-width: 85%; padding: 8px 12px; border-radius: 12px; background: var(--bubble); box-shadow: var(--shadow-bubble)">', n_p, name)
    # folded tool calls: a tool block
    t, n = re.subn(r'(aria-expanded="false" style="[^"]*?)border: 1px solid var\(--line\); border-radius: \d+px; background: transparent',
                   r'\1align-self: flex-start; width: 85%; border: 0; border-radius: 8px; background: var(--code); box-shadow: inset 0 0 0 1px var(--line)', t)
    assert n == n_tool, (name, "tool", n)
    t = t.replace("border-radius: 4px; background: var(--sunken)\">registry", "border-radius: 4px; background: var(--well)\">registry")
    return t


def main_page(t, name):
    t = desktop(t, name, "lead / cluster ops / k-rollout", "chat")
    t = sub1(t, '<div style="font: 11px/1.4 var(--mono); color: var(--text3)">Lead / cluster ops</div>\n', "", name=name)
    t = chat(t, name, 3, 2)
    # composer: the canvas input (tint + inset ring), the turn state as a phase pill
    t = sub1(t, 'padding: 6px 6px 6px 8px; border: 1px solid var(--line2); border-radius: 12px; background: var(--surface)',
             'padding: 6px 6px 6px 8px; border: 0; border-radius: 10px; background: var(--input); box-shadow: inset 0 0 0 1px var(--line)', name=name)
    t = sub1(t, "<span>turn paused until you answer</span>",
             '<span style="padding: 1px 9px; border-radius: 999px; color: var(--warn); background: color-mix(in srgb, var(--warn) 20%, transparent)">turn paused until you answer</span>', name=name)
    return t


def board_page(t, name):
    t = desktop(t, name, "lead / board", "html")
    t = sub1(t, '<div style="font: 11px/1.4 var(--mono); color: var(--text3)">Lead</div>\n', "", name=name)
    t, n = re.subn(r'<span style="display: flex; align-items: center; gap: 7px; height: 30px;[^\n]*html view</span>\n', "", t)
    assert n == 1, name
    # the agent's own view keeps an alpha channel: no opaque ground of its own
    t = sub1(t, "border-top: 1px solid var(--line); background: var(--surface)\">", "border-top: 1px solid var(--line); background: transparent\">", name=name)
    return t


def file_page(t, name):
    t = desktop(t, name, "lead / cluster ops / k-rollout / deploy/api / api.yaml", "file")
    t = sub1(t, '<div style="font: 11px/1.4 var(--mono); color: var(--text3)">Lead / cluster ops / k-rollout / deploy/api</div>\n', "", name=name)
    t, n = re.subn(r'<span style="[^"\n]*">file view</span>\n', "", t)
    assert n == 1, name
    t = sub1(t, "background: var(--bg); font: 12.5px/1.65 var(--mono)", "background: var(--well); font: 12.5px/1.65 var(--mono)", name=name)
    return t


FLAT = "background: linear-gradient(var(--panel-flat), var(--panel-flat)), var(--page)"


def phone_chat(t, name):
    t = sub1(t, "padding-top: 44px; background: var(--surface); color", "padding-top: 44px; " + FLAT + "; color", name=name)
    strip = head('<span style="opacity: .55">lead / cluster ops / </span>k-rollout', '<span style="%s">chat</span>' % EXT)
    strip = strip.replace("height: 32px; padding: 0 6px", "height: 28px; padding: 0 12px")
    t = sub1(t, '\n<header style="display: flex; align-items: center; gap: 10px; padding: 4px 12px 10px 6px;', '\n' + strip + '\n<header style="display: flex; align-items: center; gap: 10px; padding: 4px 12px 10px 6px;', name=name)
    t = chat(t, name, 2, 1)
    t = sub1(t, 'padding: 4px 4px 4px 4px; border: 1px solid var(--line2); border-radius: 12px; background: var(--surface)',
             'padding: 4px 4px 4px 4px; border: 0; border-radius: 10px; background: var(--input); box-shadow: inset 0 0 0 1px var(--line)', name=name)
    return t


def phone_tree(t, name):
    return sub1(t, "padding: 44px 12px 0; background: var(--side); color", "padding: 44px 12px 0; " + FLAT + "; color", name=name)


def states(t, name):
    t = sub1(t, "padding: 36px; background: var(--bg); color", "padding: 36px; background: var(--page); color", name=name)
    # a card that shows a theme carries that theme's own ground under its panel
    t, n = re.subn(r'(<div data-theme="\{\{th\.name\}\}" style="[^"]*?)border-radius: \d+px; background: var\(--surface\)',
                   r'\1border-radius: 16px; background: linear-gradient(var(--panel), var(--panel)), var(--page); box-shadow: var(--shadow-bezel)', t)
    assert n >= 1, name
    t, n2 = re.subn(r'border: 1px solid var\(--line\); border-radius: 12px; background: var\(--(surface|side)\)',
                    'border: 1px solid var(--line); border-radius: 16px; background: var(--panel); box-shadow: var(--shadow-bezel)', t)
    print("states: theme cards %d, panel cards %d" % (n, n2))
    return t


PAGES = {"Main": main_page, "Board": board_page, "File": file_page, "Phone-Chat": phone_chat, "Phone-Tree": phone_tree, "States": states}
for stem, fn in PAGES.items():
    src = (BACKUP / (stem + ".dc.html")).read_text()
    out = fn(common(src, stem), stem)
    (OUT / (stem + ".dc.html")).write_text(out)
    left = re.findall(r"background: var\(--(?:bg|side|surface)\)", out)
    print("%-11s ok; opaque-era grounds left: bg %d, side %d, surface %d" % (stem, left.count("background: var(--bg)"), left.count("background: var(--side)"), left.count("background: var(--surface)")))
