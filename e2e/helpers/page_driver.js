// Drives the team's served dispatcher page without a browser: it fetches the page
// from a running team, runs the page's own script in a vm on a small stub DOM, and
// bridges `fantastic.send` and `fantastic.watch` to the live /team_ws/ws socket with
// the same frames as web/page_shim.js. The stub DOM is the El pattern of
// crates/team/tests/js/chat_harness.js. Needs node >= 22 (a global WebSocket).
//
//   node page_driver.js PORT
//
// Commands come as JSON lines on stdin, one JSON line answers each on stdout:
//   {cmd:'load'}                       fetch the page, run its script, wait for the sidebar
//   {cmd:'state'}                      what is drawn (see snapshot)
//   {cmd:'click_expander', worker}     click that worker's expander row
//   {cmd:'click_node', id}             click that worker or subagent row
//   {cmd:'click_back'}                 click Back
//   {cmd:'submit', text}               type text into the open chat's composer and submit
//   {cmd:'call', target, payload}      one call of the probe's own over the page's socket
//   {cmd:'frames', since}              the call/watch frames sent from index `since`
//   {cmd:'quit'}
'use strict';
if (typeof WebSocket === 'undefined') {
  process.stdout.write(JSON.stringify({ fatal: 'node too old: no global WebSocket (needs node >= 22)' }) + '\n');
  process.exit(3);
}
const vm = require('vm');
const readline = require('readline');

const port = process.argv[2];

class El {
  constructor(tag) {
    this.tag = tag; this.children = []; this.parent = null; this._text = '';
    this.dataset = {}; this.listeners = {}; this.attrs = {}; this._cls = new Set();
    this.hidden = false; this.disabled = false; this.value = ''; this.checked = false;
    this.files = null; this.scrollTop = 0; this.scrollHeight = 0; this.id = '';
  }
  get className() { return Array.from(this._cls).join(' '); }
  set className(v) { this._cls = new Set(String(v).split(/\s+/).filter(Boolean)); }
  get classList() {
    const s = this._cls;
    return { add: (...c) => c.forEach((x) => s.add(x)), remove: (...c) => c.forEach((x) => s.delete(x)), contains: (x) => s.has(x) };
  }
  get textContent() { return this._text + this.children.map((c) => c.textContent).join(''); }
  set textContent(v) { this.children.forEach((c) => { c.parent = null; }); this.children = []; this._text = String(v); }
  appendChild(n) {
    if (n.tag === '#frag') { n.children.splice(0).forEach((k) => { k.parent = null; this.appendChild(k); }); return n; }
    if (n.parent) n.remove();
    n.parent = this; this.children.push(n); return n;
  }
  prepend(n) {
    const kids = n.tag === '#frag' ? n.children.splice(0) : [n];
    kids.forEach((k) => { if (k.parent) k.remove(); k.parent = this; });
    this.children.unshift(...kids);
  }
  remove() {
    if (!this.parent) return;
    const i = this.parent.children.indexOf(this);
    if (i >= 0) this.parent.children.splice(i, 1);
    this.parent = null;
  }
  addEventListener(t, f) { (this.listeners[t] = this.listeners[t] || []).push(f); }
  dispatch(t, ev) {
    const e = Object.assign({ target: this, preventDefault() {} }, ev || {});
    (this.listeners[t] || []).forEach((f) => f(e));
  }
  setAttribute(k, v) { this.attrs[k] = String(v); }
  getAttribute(k) { return this.attrs[k]; }
  matches(sel) {
    let m;
    if ((m = /^\[data-([a-z-]+)\]$/.exec(sel))) {
      const k = m[1].replace(/-([a-z])/g, (_, c) => c.toUpperCase());
      return this.dataset[k] !== undefined;
    }
    if ((m = /^([a-z0-9]*)((?:\.[A-Za-z0-9_-]+)*)$/.exec(sel))) {
      if (m[1] && this.tag !== m[1]) return false;
      return m[2].split('.').filter(Boolean).every((c) => this._cls.has(c)) && (m[1] || m[2]);
    }
    return false;
  }
  closest(sel) { for (let n = this; n; n = n.parent) if (n.matches(sel)) return n; return null; }
  all(sel) {
    const out = [];
    const walk = (n) => n.children.forEach((c) => { if (c.matches(sel)) out.push(c); walk(c); });
    walk(this);
    return out;
  }
  querySelector(sel) { return this.all(sel)[0] || null; }
  querySelectorAll(sel) { return this.all(sel); }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const frames = [];      // every call/watch/unwatch frame the page sent, in order
let pendingCalls = 0;   // calls sent and not yet answered
let rt = null;

// The page's `fantastic`: send and watch over the real socket, as page_shim.js does.
function bridge() {
  const ws = new WebSocket('ws://127.0.0.1:' + port + '/team_ws/ws');
  const queued = [];
  const calls = {};
  const watchers = {};
  let next = 1;
  const out = (frame) => {
    const text = JSON.stringify(frame);
    if (ws.readyState === 1) ws.send(text); else queued.push(text);
  };
  ws.onopen = () => queued.splice(0).forEach((t) => ws.send(t));
  ws.onmessage = (m) => {
    let f;
    try { f = JSON.parse(m.data); } catch (e) { return; }
    if (f.type === 'reply' || f.type === 'error') {
      const c = calls[f.id];
      if (!c) return;
      delete calls[f.id];
      pendingCalls--;
      c.entry.reply = f.type === 'reply' ? f.data : { error: String(f.error || 'refused') };
      if (f.type === 'reply') c.resolve(f.data); else c.reject(new Error(String(f.error || 'the call was refused')));
    } else if (f.type === 'event') {
      (watchers[f.src] || []).slice().forEach((cb) => { try { cb(f.payload); } catch (e) { /* one listener never stops the rest */ } });
    }
  };
  return {
    opened: new Promise((res, rej) => { ws.addEventListener('open', res); ws.addEventListener('error', () => rej(new Error('socket error'))); }),
    close: () => ws.close(),
    send(target, payload) {
      return new Promise((resolve, reject) => {
        const id = 'p' + next++;
        const entry = { frame: 'call', target, payload: JSON.parse(JSON.stringify(payload)) };
        calls[id] = { resolve, reject, entry };
        pendingCalls++;
        frames.push(entry);
        out({ type: 'call', target, payload, id });
      });
    },
    watch(src, cb) {
      const list = watchers[src] || (watchers[src] = []);
      if (list.length === 0) { frames.push({ frame: 'watch', target: src }); out({ type: 'watch', src }); }
      list.push(cb);
      return () => {
        const l = watchers[src] || [];
        const i = l.indexOf(cb);
        if (i >= 0) l.splice(i, 1);
        if (l.length === 0) { frames.push({ frame: 'unwatch', target: src }); out({ type: 'unwatch', src }); }
      };
    },
  };
}

async function load() {
  const res = await fetch('http://127.0.0.1:' + port + '/team_page/file/index.html');
  const html = await res.text();
  const scripts = Array.from(html.matchAll(/<script>([\s\S]*?)<\/script>/g)).map((m) => m[1]);
  const script = scripts[scripts.length - 1];
  const byId = {};
  const idRe = /<[a-z0-9]+[^>]*\sid="([^"]+)"/g;
  let m;
  while ((m = idRe.exec(html))) { byId[m[1]] = new El('div'); byId[m[1]].id = m[1]; }
  byId.back.hidden = true;
  byId.pane.hidden = true;
  const document = {
    getElementById: (id) => byId[id] || null,
    createElement: (tag) => new El(tag),
    createDocumentFragment: () => new El('#frag'),
  };
  const store = new Map();
  const scrolls = [];
  const fantastic = bridge();
  await fantastic.opened;
  const sandbox = {
    document,
    window: { scrollTo: (x, y) => scrolls.push(y), scrollY: 0, addEventListener() {}, confirm: () => true, prompt: () => null },
    fantastic,
    FileReader: class {},
    localStorage: {
      getItem: (k) => (store.has(k) ? store.get(k) : null),
      setItem: (k, v) => store.set(k, String(v)),
      removeItem: (k) => store.delete(k),
    },
    setTimeout, clearTimeout, console,
  };
  const ctx = vm.createContext(sandbox);
  vm.runInContext(script, ctx);
  rt = { byId, ctx, store, scrolls, js: (code) => vm.runInContext(code, ctx), html };
  return { html_has_renderSidebar: html.includes('function renderSidebar('), html_has_shim: html.includes('window.fantastic = {') };
}

// Wait until every call has been answered and nothing new was sent for a while.
async function settle(quietMs = 250, maxMs = 15000) {
  const end = Date.now() + maxMs;
  let last = -1;
  let quiet = 0;
  while (Date.now() < end) {
    await sleep(40);
    const n = frames.length;
    if (pendingCalls === 0 && n === last) { quiet += 40; if (quiet >= quietMs) return; } else { quiet = 0; }
    last = n;
  }
}

function snapshot() {
  const nav = rt.byId.sidebar;
  const rows = nav.children
    .filter((c) => c.matches('.item'))
    .map((c) => ({
      cls: c.className, node: c.dataset.node || null, expander: c.dataset.expander || null,
      aria: c.attrs['aria-expanded'] === undefined ? null : c.attrs['aria-expanded'],
      text: c.textContent, parent: c.parent && c.parent.id,
    }));
  const chat = rt.js('chat ? { node: chat.node, placeholder: chat.input.placeholder, rows: chat.transcript.children.map((c) => c.textContent) } : null');
  return {
    rows, groupTitles: nav.children.filter((c) => c.matches('.group-title')).map((c) => c.textContent),
    sidebarHidden: nav.hidden, paneHidden: rt.byId.pane.hidden, backHidden: rt.byId.back.hidden,
    title: rt.byId.title.textContent, error: rt.byId.error.textContent,
    chat, expanded: rt.js('Array.from(expanded)'), stackDepth: rt.js('stack.length'),
    scrolls: rt.scrolls.slice(), remembered: Object.fromEntries(rt.store),
    frameCount: frames.length,
  };
}

function rowFor(pred) {
  return rt.byId.sidebar.children.find((c) => c.matches('.item') && pred(c));
}

async function run(c) {
  switch (c.cmd) {
    case 'load': {
      const info = await load();
      await settle();
      return info;
    }
    case 'state': return snapshot();
    case 'click_expander': {
      const row = rowFor((r) => r.dataset.expander === c.worker);
      if (!row) return { error: 'no expander row for ' + c.worker };
      rt.byId.sidebar.dispatch('click', { target: row });
      await settle();
      return snapshot();
    }
    case 'click_node': {
      const row = rowFor((r) => r.dataset.node === c.id);
      if (!row) return { error: 'no row for ' + c.id };
      rt.byId.sidebar.dispatch('click', { target: row });
      await settle();
      return snapshot();
    }
    case 'click_back': {
      if (rt.byId.back.hidden) return { error: 'Back is hidden' };
      rt.byId.back.dispatch('click');
      await settle();
      return snapshot();
    }
    case 'submit': {
      const chat = rt.js('chat');
      if (!chat) return { error: 'no chat is open' };
      chat.input.value = c.text;
      chat.input.parent.dispatch('submit');
      await settle(c.quiet_ms || 600);
      return snapshot();
    }
    case 'call': {   // a call of the probe's own over the same socket (not the page's)
      const r = await rt.ctx.fantastic.send(c.target, c.payload).catch((e) => ({ error: String(e.message || e) }));
      return { reply: r };
    }
    case 'frames': return { frames: frames.slice(c.since || 0), total: frames.length };
    default: return { error: 'unknown command ' + c.cmd };
  }
}

const rl = readline.createInterface({ input: process.stdin });
let chain = Promise.resolve();
rl.on('line', (line) => {
  chain = chain.then(async () => {
    let c;
    try { c = JSON.parse(line); } catch (e) { process.stdout.write(JSON.stringify({ error: 'bad json' }) + '\n'); return; }
    if (c.cmd === 'quit') { process.exit(0); }
    let out;
    try { out = await run(c); } catch (e) { out = { error: String(e && e.stack || e) }; }
    process.stdout.write(JSON.stringify(out) + '\n');
  });
});
rl.on('close', () => process.exit(0));
