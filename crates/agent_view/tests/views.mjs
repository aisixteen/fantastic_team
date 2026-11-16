// A team agent's html view, run WHOLE on fantastic's view harness (`fantastic_tui::VIEW_HARNESS`,
// in the checkout Cargo pins for `fantastic-tui`: found with `cargo metadata`, no copy): its
// recorded sends and watches, the copy of every call mirrored to the target's watchers as the
// kernel does, its timers. The team adds only what a team view needs on top of it: its template
// filled the way `team_agent_view::page` fills it (`{{helpers}}`, `{{agent}}`, any other
// placeholder given), `window.fantastic`, and a small stand-in DOM whose text the tests read
// (the harness's own elements are loose: they keep what is set on them, never what is put in
// them).
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = fileURLToPath(new URL("../../../", import.meta.url));
const meta = JSON.parse(execFileSync("cargo", ["metadata", "--format-version", "1", "--locked"], {
  cwd: ROOT, encoding: "utf8", maxBuffer: 1 << 28,
}));
const tui = meta.packages.find((p) => p.name === "fantastic-tui");
if (!tui) throw new Error("cargo metadata names no fantastic-tui package");
const TUI = dirname(tui.manifest_path);
const AT = /pub const VIEW_HARNESS: &str = "([^"]+)";/.exec(readFileSync(join(TUI, "src", "lib.rs"), "utf8"));
if (!AT) throw new Error("fantastic-tui declares no VIEW_HARNESS");
const base = await import(pathToFileURL(join(TUI, AT[1])).href);
export const { clocksReadNothing, ownCopiesReadNothing } = base;

const HELPERS = readFileSync(fileURLToPath(new URL("../src/helpers.js", import.meta.url)), "utf8");
const STASH = "__team_dom";

// Runs inside the view's context, before the view's own script: the stand-in DOM replaces the
// harness's loose document (kept on a loose element the test reaches through `byId`).
function installDom(g) {
  class Node {
    constructor(tag, doc) {
      this.tagName = String(tag).toUpperCase();
      this.ownerDocument = doc;
      this.childNodes = [];
      this.attrs = {};
      this.listeners = {};
      this.style = { setProperty() {} };
      this.className = "";
      this.value = "";
      this.hidden = false;
      this._text = "";
    }
    appendChild(c) { this.childNodes.push(c); c.parentNode = this; return c; }
    get textContent() { return this._text + this.childNodes.map((c) => c.textContent).join(""); }
    set textContent(t) { this.childNodes = []; this._text = String(t); }
    setAttribute(k, v) { this.attrs[k] = String(v); }
    getAttribute(k) { return k in this.attrs ? this.attrs[k] : null; }
    removeAttribute(k) { delete this.attrs[k]; }
    addEventListener(type, fn) { (this.listeners[type] ||= []).push(fn); }
    dispatch(type) { (this.listeners[type] || []).forEach((fn) => fn({ type, target: this })); }
    querySelector() { return null; }
    querySelectorAll() { return []; }
    focus() { this.ownerDocument.activeElement = this; }
    blur() { if (this.ownerDocument.activeElement === this) this.ownerDocument.activeElement = null; }
  }
  const byId = new Map();
  const doc = {
    activeElement: null,
    createElement: (tag) => new Node(tag, doc),
    createElementNS: (_ns, tag) => new Node(tag, doc),
    createTextNode: (t) => ({ textContent: String(t) }),
    getElementById: (id) => {
      if (!byId.has(id)) byId.set(id, new Node("div", doc));
      return byId.get(id);
    },
  };
  doc.documentElement = new Node("html", doc);
  doc.body = new Node("body", doc);
  g.document.getElementById("__team_dom").doc = doc;
  g.document = doc;
  g.window.fantastic = g.fantastic;
}

/**
 * Run the view `file` as agent `agent` on fantastic's harness; `reply(target, payload)` answers
 * each send, `fill` fills more placeholders. Returns the harness (`sends` [{target, p}],
 * `watches`, `intervals`, `settle`, `publish`, `since`) and `document`, the stand-in DOM.
 */
export function runView(file, agent, reply, fill = {}) {
  const html = readFileSync(file, "utf8").replace("<script>", `<script>(${installDom.toString()})(globalThis);\n`);
  const h = base.run(html, { helpers: HELPERS, agent, ...fill }, reply);
  h.document = h.byId.get(STASH).doc;
  return h;
}

/** The verbs (`p.type`) of the sends made to `target` since `mark`. */
export function verbsSince(h, mark, target) {
  return h.since(mark).filter((s) => s.target === target).map((s) => s.p.type);
}

/** An event on `src`'s inbox handed to each of the view's watchers now, without waiting for
 *  what it starts (`publish` waits; this does not, and a source nobody watches is no error). */
export function deliver(h, src, ev) {
  (h.watches.get(src) ?? []).slice().forEach((cb) => cb(JSON.parse(JSON.stringify(ev))));
}

/** The sources the view watches, each with how many watchers it holds on it. */
export function watched(h) {
  return [...h.watches].filter(([, l]) => l.length > 0).map(([src, l]) => [src, l.length]);
}

/** Other clients' calls to the view's agents, mirrored to its watchers: how many sends they cost
 *  (0 when the view keeps the rule). */
export async function sendsOnOthers(h, others) {
  await h.settle();
  const mark = h.sends.length;
  for (const [src, p] of others) deliver(h, src, p);
  await h.settle();
  return h.sends.length - mark;
}
