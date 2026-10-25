// Runs the dispatcher page's script against a stub DOM and a scripted `fantastic`,
// and prints one JSON object for the Rust unit tests to assert on.
//   node chat_harness.js <path/to/dispatcher.html> <scenario>
'use strict';
const fs = require('fs');
const vm = require('vm');

const htmlPath = process.argv[2];
const scenarioName = process.argv[3];
const html = fs.readFileSync(htmlPath, 'utf8');
const script = /<script>([\s\S]*?)<\/script>/.exec(html)[1];

class El {
  constructor(tag) {
    this.tag = tag;
    this.children = [];
    this.parent = null;
    this._text = '';
    this.dataset = {};
    this.listeners = {};
    this.attrs = {};
    this._cls = new Set();
    this.hidden = false;
    this.disabled = false;
    this.value = '';
    this.checked = false;
    this.files = null;
    this.scrollTop = 0;
    this.scrollHeight = 0;
    this.id = '';
  }
  get className() { return Array.from(this._cls).join(' '); }
  set className(v) { this._cls = new Set(String(v).split(/\s+/).filter(Boolean)); }
  get classList() {
    const s = this._cls;
    return {
      add: (...c) => c.forEach((x) => s.add(x)),
      remove: (...c) => c.forEach((x) => s.delete(x)),
      contains: (x) => s.has(x),
    };
  }
  get textContent() { return this._text + this.children.map((c) => c.textContent).join(''); }
  set textContent(v) {
    this.children.forEach((c) => { c.parent = null; });
    this.children = [];
    this._text = String(v);
  }
  appendChild(n) {
    if (n.tag === '#frag') {
      n.children.splice(0).forEach((k) => { k.parent = null; this.appendChild(k); });
      return n;
    }
    if (n.parent) n.remove();
    n.parent = this;
    this.children.push(n);
    return n;
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
  click() { if (!this.disabled) this.dispatch('click'); }
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
  closest(sel) {
    for (let n = this; n; n = n.parent) if (n.matches(sel)) return n;
    return null;
  }
  all(sel) {
    const out = [];
    const walk = (n) => n.children.forEach((c) => { if (c.matches(sel)) out.push(c); walk(c); });
    walk(this);
    return out;
  }
  querySelector(sel) { return this.all(sel)[0] || null; }
  querySelectorAll(sel) { return this.all(sel); }
}

function makeRuntime(handler) {
  const calls = [];
  const watchers = [];
  const timers = new Map();
  let nextTimer = 1;
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
  class FileReader {
    readAsText(f) {
      Promise.resolve().then(() => {
        if (f.failRead) { if (this.onerror) this.onerror(new Error('read')); return; }
        this.result = f.content;
        if (this.onload) this.onload();
      });
    }
  }
  const fantastic = {
    send(id, payload) {
      calls.push({ id, payload: JSON.parse(JSON.stringify(payload)) });
      return Promise.resolve(handler(id, payload));
    },
    watch(node, cb) {
      watchers.push({ node, cb });
      return () => { const i = watchers.findIndex((w) => w.cb === cb); if (i >= 0) watchers.splice(i, 1); };
    },
  };
  const store = new Map();
  const sandbox = {
    document,
    window: { scrollTo() {}, scrollY: 0, addEventListener() {}, confirm: () => true },
    fantastic,
    FileReader,
    localStorage: {
      getItem: (k) => (store.has(k) ? store.get(k) : null),
      setItem: (k, v) => store.set(k, String(v)),
      removeItem: (k) => store.delete(k),
    },
    setTimeout: (f, ms) => { const id = nextTimer++; timers.set(id, { f, ms }); return id; },
    clearTimeout: (id) => { timers.delete(id); },
    console,
  };
  const ctx = vm.createContext(sandbox);
  vm.runInContext(script, ctx);
  const rt = {
    calls, watchers, byId, ctx, timers,
    js: (code) => vm.runInContext(code, ctx),
    async flush() { for (let i = 0; i < 6; i++) await new Promise((r) => setImmediate(r)); },
    async runTimers() {
      const due = Array.from(timers.entries());
      timers.clear();
      due.forEach(([, t]) => t.f());
      await rt.flush();
    },
    async open(node) {
      rt.js(`showChat({cell: {label: 'x', kind: 'chat', target: ${JSON.stringify(node)}}}, ${JSON.stringify(node)})`);
      await rt.flush();
      return rt.chat();
    },
    chat: () => rt.js('chat'),
    client: () => rt.js('chatClient'),
    // an event as the agent emits it for this page's client
    async emit(ev, other) {
      const e = Object.assign({ client_id: other ? 'someone_else' : rt.client() }, ev);
      watchers.slice().forEach((w) => w.cb(e));
      await rt.flush();
    },
    msgs() { return rt.chat().transcript.children.map((c) => ({ cls: c.className, text: c.textContent })); },
    sent(type) { return calls.filter((c) => c.payload.type === type); },
  };
  return rt;
}

const base = (extra) => (id, p) => {
  if (p.type === 'history') return { rows: [] };
  if (p.type === 'sidebar') return {};
  return (extra && extra(id, p)) || {};
};

const scenarios = {
  async text_once() {
    const rt = makeRuntime(base());
    await rt.open('w1');
    await rt.emit({ type: 'part_start', stream_id: 's1', kind: 'text' });
    await rt.emit({ type: 'part_delta', stream_id: 's1', text: 'a' });
    await rt.emit({ type: 'part_delta', stream_id: 's1', text: 'b' });
    await rt.emit({ type: 'part_end', stream_id: 's1' });
    // another client's whole stream is ignored
    await rt.emit({ type: 'part_start', stream_id: 'x1', kind: 'text' }, true);
    await rt.emit({ type: 'part_delta', stream_id: 'x1', text: 'zzz' }, true);
    await rt.emit({ type: 'part_end', stream_id: 'x1' }, true);
    return { msgs: rt.msgs() };
  },

  async tool_block() {
    const rt = makeRuntime(base());
    const c = await rt.open('w1');
    const snap = () => c.transcript.all('.tool-block').map((b) => ({ text: b.textContent, state: b.dataset.state }));
    await rt.emit({ type: 'status', phase: 'tool_calling', detail: { tool: { call_id: 'c1', verb: 'git', target: 'g' } } });
    const first = snap();
    await rt.emit({ type: 'status', phase: 'tool_calling', detail: { tool: { call_id: 'c1', verb: 'git', target: 'g', reply_preview: 'all fine' } } });
    const second = snap();
    await rt.emit({ type: 'status', phase: 'tool_calling', detail: { tool: { call_id: 'c2', verb: 'ls', target: 'd' } } });
    return { first, second, third: snap() };
  },

  async inquiries() {
    const state = { list: [], answer: { answered: 5 } };
    const rt = makeRuntime(base((id, p) => {
      if (id === 'user' && p.type === 'inquiries') return { inquiries: state.list };
      if (id === 'user' && p.type === 'answer_inquiry') return state.answer;
      return null;
    }));
    const c = await rt.open('w1');
    const onOpen = rt.sent('inquiries').length;
    state.list = [
      { re: 5, text: 'Proceed?', kind: 'confirm', asker: 'w1' },
      { re: 6, text: 'Which?', kind: 'single', options: ['alpha', 'beta', 'gamma'], asker: 'w1' },
    ];
    await rt.emit({ type: 'done', source: 'w1' });
    const beforeTimer = rt.sent('inquiries').length;
    await rt.runTimers();
    const asked = rt.sent('inquiries').map((x) => x.payload);
    const card = (re) => c.transcript.all('.inquiry').find((q) => q.dataset.re === String(re));
    const labels = (re) => card(re).all('button').map((b) => b.textContent);
    const out = { onOpen, beforeTimer, asked, labels5: labels(5), labels6: labels(6) };
    // an error reply re-enables the buttons and shows the error
    state.answer = { error: 'user: no open inquiry 5' };
    card(5).all('button')[0].click();
    out.disabledWhileSending = card(5).all('button').map((b) => b.disabled);
    await rt.flush();
    out.afterError = {
      disabled: card(5).all('button').map((b) => b.disabled),
      error: card(5).querySelector('.inq-error').textContent,
    };
    // a good reply closes the card with the answer
    state.answer = { answered: 5 };
    card(5).all('button')[0].click();
    await rt.flush();
    out.afterOk = card(5).textContent;
    out.answers = rt.sent('answer_inquiry').map((x) => ({ id: x.id, payload: x.payload }));
    // the same re is not drawn twice
    await rt.emit({ type: 'done', source: 'w1' });
    await rt.runTimers();
    out.cards = c.transcript.all('.inquiry').length;
    // single: one answer by option
    card(6).all('button')[1].click();
    await rt.flush();
    out.singleAnswer = rt.sent('answer_inquiry').pop().payload;
    return out;
  },

  async inquiry_kinds() {
    const state = { list: [
      { re: 7, text: 'Pick some', kind: 'multi', options: ['a', 'b', 'c'] },
      { re: 8, text: 'Name?', kind: 'text' },
    ] };
    const rt = makeRuntime(base((id, p) => {
      if (id === 'user' && p.type === 'inquiries') return { inquiries: state.list };
      if (id === 'user' && p.type === 'answer_inquiry') return { answered: p.re };
      return null;
    }));
    const c = await rt.open('w1');
    const card = (re) => c.transcript.all('.inquiry').find((q) => q.dataset.re === String(re));
    const boxes = card(7).all('input');
    boxes[0].checked = true;
    boxes[2].checked = true;
    card(7).all('button')[0].click();
    const text = card(8).all('input')[0];
    text.value = 'Ada';
    card(8).all('button')[0].click();
    await rt.flush();
    return { answers: rt.sent('answer_inquiry').map((x) => x.payload), multiDone: card(7).textContent };
  },

  async stats() {
    const rt = makeRuntime(base());
    const c = await rt.open('w1');
    const out = {};
    await rt.emit({ type: 'status', phase: 'done', detail: { completion_tokens: 50, gen_seconds: 2, prompt_tokens: 12000, context_window: 32000 } });
    out.plain = c.stats.textContent;
    await rt.emit({ type: 'status', phase: 'done', detail: { completion_tokens: 50, gen_seconds: 2, prompt_tokens: 12000, prompt_tokens_estimated: true, context_window: 32000 } });
    out.estimated = c.stats.textContent;
    return out;
  },

  async attach() {
    const rt = makeRuntime(base());
    const c = await rt.open('w1');
    const form = rt.byId.pane.all('form')[0];
    const out = {};
    out.hasFileInput = c.file.tag === 'input' && c.file.type === 'file';
    c.file.files = [{ name: 'a.txt', size: 5, content: 'hello' }];
    c.file.dispatch('change');
    await rt.flush();
    out.chip = c.chip.textContent;
    c.input.value = 'see';
    form.dispatch('submit');
    await rt.flush();
    const sends = rt.sent('send');
    out.keys = Object.keys(sends[0].payload).sort();
    out.text = sends[0].payload.text;
    out.bubble = c.transcript.all('.msg.user')[0].textContent;
    out.chipAfter = c.chip.textContent;
    await rt.emit({ type: 'done', source: 'w1' });
    // an attachment alone is allowed
    c.file.files = [{ name: 'b.txt', size: 2, content: 'yo' }];
    c.file.dispatch('change');
    await rt.flush();
    form.dispatch('submit');
    await rt.flush();
    out.alone = rt.sent('send')[1].payload.text;
    await rt.emit({ type: 'done', source: 'w1' });
    // a file over the cap sends nothing and says why
    const before = rt.sent('send').length;
    c.file.files = [{ name: 'big.txt', size: 200001, content: 'x' }];
    c.file.dispatch('change');
    await rt.flush();
    out.bigError = rt.byId.error.textContent;
    out.bigChip = c.chip.textContent;
    out.bigAttached = c.attached;
    out.sendsAfterBig = rt.sent('send').length - before;
    // a read failure also attaches nothing
    c.file.files = [{ name: 'bad.txt', size: 3, failRead: true }];
    c.file.dispatch('change');
    await rt.flush();
    out.readError = rt.byId.error.textContent;
    out.badAttached = c.attached;
    return out;
  },

  async reasoning() {
    const rt = makeRuntime(base());
    const c = await rt.open('w1');
    await rt.emit({ type: 'part_start', stream_id: 't1', kind: 'thinking' });
    await rt.emit({ type: 'part_delta', stream_id: 't1', text: 'let me ' });
    await rt.emit({ type: 'part_delta', stream_id: 't1', text: 'think' });
    await rt.emit({ type: 'part_end', stream_id: 't1' });
    await rt.emit({ type: 'part_start', stream_id: 't2', kind: 'thinking' });
    await rt.emit({ type: 'part_end', stream_id: 't2' });
    await rt.emit({ type: 'part_start', stream_id: 't3', kind: 'thinking' });
    await rt.emit({ type: 'part_delta', stream_id: 't3', text: 'cut' });
    await rt.emit({ type: 'part_abort', stream_id: 't3', reason: 'cancel' });
    await rt.emit({ type: 'part_start', stream_id: 'k1', kind: 'call' });
    await rt.emit({ type: 'part_delta', stream_id: 'k1', text: '<call/>' });
    await rt.emit({ type: 'part_end', stream_id: 'k1' });
    return {
      blocks: c.transcript.all('details').map((d) => ({
        cls: d.className, summary: d.children[0].textContent, body: d.children[1].textContent,
      })),
      assistant: c.transcript.all('.msg.assistant').length,
      total: c.transcript.children.length,
    };
  },

  async status_replay() {
    const run = async (current, pending) => {
      const rt = makeRuntime(base((id, p) => (p.type === 'status' ? { current, mine_pending: pending || [] } : null)));
      const c = await rt.open('w1');
      const out = {
        statusCall: rt.sent('status').map((x) => x.payload),
        msgs: rt.msgs(),
        tools: c.transcript.all('.tool-block').length,
        disabled: c.send.disabled,
      };
      await rt.emit({ type: 'done', source: 'w1' });
      out.disabledAfterDone = c.send.disabled;
      return out;
    };
    const mine = await run({ is_mine: true, text: 'q', text_so_far: 'par', phase: 'streaming', last_tool: { call_id: 'c1', verb: 'git', target: 'g', reply_preview: 'ok' } },
      [{ send_id: 2, text: 'next one' }]);
    const notMine = await run({ is_mine: false, phase: 'streaming' });
    const none = await run(null);
    return { mine, notMine, none };
  },

  async status_error() {
    const rt = makeRuntime(base((id, p) => (p.type === 'status' ? { error: 'nope' } : null)));
    await rt.open('w1');
    return { msgs: rt.msgs(), disabled: rt.chat().send.disabled };
  },

  async rows() {
    const rt = makeRuntime(base());
    const draw = (item) => {
      rt.ctx.__item = item;
      rt.js("renderSidebar({team: 't', groups: [], ungrouped: [__item]})");
      const row = rt.byId.sidebar.all('.worker')[0];
      return { text: row.textContent, minutes: row.querySelector('.minutes') ? row.querySelector('.minutes').textContent : null, pr: row.querySelector('.pr') ? row.querySelector('.pr').textContent : null };
    };
    return {
      full: draw({ kind: 'worker', id: 'w1', state: 'busy', task: 'r#1', busy: true, minutes: 2, pr: 12 }),
      empty: draw({ kind: 'worker', id: 'w1', state: 'idle', task: null, busy: false, minutes: null, pr: null }),
      zero: draw({ kind: 'worker', id: 'w1', state: 'busy', task: 'r#1', busy: true, minutes: 0, pr: null }),
    };
  },
};

scenarios[scenarioName]().then(
  (out) => { process.stdout.write(JSON.stringify(out)); },
  (err) => { process.stderr.write(String(err && err.stack ? err.stack : err)); process.exit(1); },
);
