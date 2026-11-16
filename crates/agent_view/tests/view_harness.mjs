// A view of a team agent, run WHOLE in a bare vm context: its template filled the way
// `team_agent_view::page` fills it (`{{helpers}}`, `{{agent}}`, and any other placeholder
// given), its scripts run against a small stand-in DOM and a stand-in `fantastic` connector
// that records every send and watch. The timers are recorded, never run: a view that sets
// one is a view that polls. Used by each view's `live.test.mjs` (one model, many
// view-models: a view follows its agent's events, it never re-reads on a clock).
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const HELPERS = readFileSync(fileURLToPath(new URL("../src/helpers.js", import.meta.url)), "utf8");

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

function makeDocument() {
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
  return doc;
}

/**
 * Run the view `file` as agent `agent`. `reply(target, payload)` answers each send (a value
 * or a promise). `fill` fills more placeholders ({name: text}). Returns:
 *   sends     [{target, payload}] every send the view made, in order
 *   watches   [{src}] every watch the view asked for
 *   timers    [{kind, ms}] every setInterval / setTimeout the view set
 *   deliver(src, payload)  an event on `src`'s inbox reaches each of the view's watchers
 *   settle()  let the view's pending replies land
 *   document  the stand-in document
 */
export function runView(file, agent, reply, fill = {}) {
  let src = readFileSync(file, "utf8").split("{{helpers}}").join(HELPERS).split("{{agent}}").join(agent);
  for (const [k, v] of Object.entries(fill)) src = src.split(`{{${k}}}`).join(v);
  const scripts = [...src.matchAll(/<script>([\s\S]*?)<\/script>/g)].map((m) => m[1]);
  if (scripts.length === 0) throw new Error(`${file} has no script`);
  const sends = [];
  const watches = [];
  const timers = [];
  const watchers = new Map();
  const document = makeDocument();
  const fantastic = {
    send(target, payload) {
      sends.push({ target, payload: JSON.parse(JSON.stringify(payload)) });
      return Promise.resolve(reply(target, payload));
    },
    watch(srcId, cb) {
      watches.push({ src: srcId });
      if (!watchers.has(srcId)) watchers.set(srcId, []);
      watchers.get(srcId).push(cb);
      return () => {};
    },
  };
  const ctx = vm.createContext({
    document,
    fantastic,
    window: {},
    console,
    setInterval: (_fn, ms) => { timers.push({ kind: "setInterval", ms }); return timers.length; },
    setTimeout: (_fn, ms) => { timers.push({ kind: "setTimeout", ms }); return timers.length; },
    clearInterval() {},
    clearTimeout() {},
  });
  for (const s of scripts) vm.runInContext(s, ctx);
  return {
    sends,
    watches,
    timers,
    document,
    deliver(srcId, payload) { (watchers.get(srcId) || []).forEach((cb) => cb(payload)); },
    async settle() { for (let i = 0; i < 20; i++) await new Promise((r) => setImmediate(r)); },
  };
}

/** The verbs (`payload.type`) of the sends made to `target` since `from`. */
export function verbsSince(view, from, target) {
  return view.sends.slice(from).filter((s) => s.target === target).map((s) => s.payload.type);
}
