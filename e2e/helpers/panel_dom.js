// Drives the packs panel (crates/team/web/packs.html) against a stub DOM and the LIVE
// team: `fantastic.send(id, payload)` runs `<binary> send <item>` in the run dir, as the
// panel's host does. No dependencies.
//   node panel_dom.js <packs.html> <binary> <run_dir> <pack> <worker> <install|eval>
// Prints one JSON object: {error, result_text} (the #error and #result text).
'use strict';
const fs = require('fs');
const vm = require('vm');
const cp = require('child_process');

const [htmlPath, binary, runDir, pack, worker, mode] = process.argv.slice(2);
const html = fs.readFileSync(htmlPath, 'utf8');
const script = /<script>([\s\S]*?)<\/script>/.exec(html)[1];

class El {
  constructor(tag) {
    this.tag = tag;
    this.children = [];
    this.parent = null;
    this._text = '';
    this.listeners = {};
    this.attrs = {};
    this.className = '';
    this.value = '';
    this.id = '';
    this.type = '';
    this.placeholder = '';
  }
  get textContent() { return this._text + this.children.map((c) => c.textContent).join(''); }
  set textContent(v) {
    this.children.forEach((c) => { c.parent = null; });
    this.children = [];
    this._text = String(v);
  }
  appendChild(n) { n.parent = this; this.children.push(n); return n; }
  addEventListener(t, f) { (this.listeners[t] = this.listeners[t] || []).push(f); }
  click() { (this.listeners.click || []).forEach((f) => f({ target: this, preventDefault() {} })); }
  setAttribute(k, v) { this.attrs[k] = String(v); }
}

const byId = {};
const idRe = /<[a-z0-9]+[^>]*\sid="([^"]+)"/g;
let m;
while ((m = idRe.exec(html))) { byId[m[1]] = new El('div'); byId[m[1]].id = m[1]; }
// Elements the panel creates (drawDetail) register under their id once they are attached.
const document = {
  getElementById: (id) => {
    if (byId[id]) return byId[id];
    return findCreated(id);
  },
  createElement: (tag) => new El(tag),
};
function findCreated(id) {
  const walk = (n) => {
    if (n.id === id) return n;
    for (const c of n.children) { const f = walk(c); if (f) return f; }
    return null;
  };
  for (const root of ['detail']) { const f = walk(byId[root]); if (f) return f; }
  return null;
}

const pending = [];
const fantastic = {
  send(id, payload) {
    const p = new Promise((resolve) => {
      if (id === 'pack_listing') { resolve({ packs: [] }); return; } // the list at load; nothing to pick
      const item = JSON.stringify({ target: id, payload });
      const r = cp.spawnSync(binary, ['send', item], { cwd: runDir, env: process.env, encoding: 'utf8' });
      try { resolve(JSON.parse(r.stdout).result); } catch (e) { resolve({ error: 'bad reply: ' + r.stdout + r.stderr }); }
    });
    pending.push(p);
    return p;
  },
};
const window = { confirm: () => true };
const ctx = vm.createContext({ document, window, fantastic, console, Promise, JSON, Math, Object, Array, String, Number, isFinite });
vm.runInContext(script, ctx);

async function settle() {
  while (pending.length) await pending.shift();
  await new Promise((r) => setImmediate(r));
  while (pending.length) await pending.shift();
}

(async () => {
  await settle();
  vm.runInContext("drawDetail({name: " + JSON.stringify(pack.split('/').pop()) + ", version: '1.0.0', ports: [], grants: []})", ctx);
  document.getElementById('f-pack').value = pack;
  document.getElementById('f-into').value = 'templates';
  document.getElementById('f-worker').value = worker;
  // The panel's own buttons: `preview` is the install without approve, `eval` the eval.
  document.getElementById(mode === 'eval' ? 'eval' : 'preview').click();
  await settle();
  process.stdout.write(JSON.stringify({
    error: byId.error.textContent,
    result_text: document.getElementById('result').textContent,
  }) + '\n');
})();
