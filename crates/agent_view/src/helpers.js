// pure
function txt(v) { return v == null ? "" : String(v); }
function count(v) { return typeof v === "number" && isFinite(v) && v >= 0 ? Math.floor(v) : 0; }
function secText(v) { return typeof v === "number" && isFinite(v) ? String(Math.floor(v)) + " s" : "-"; }
function bytesText(v) {
  if (typeof v !== "number" || !isFinite(v) || v < 0) return "-";
  if (v < 1024) return String(Math.round(v)) + " B";
  if (v < 1048576) return (Math.round(v / 102.4) / 10) + " KB";
  return (Math.round(v / 104857.6) / 10) + " MB";
}
// the class of a result word: ok green, refused amber, unreachable or error red
function resultClass(r) {
  var s = txt(r);
  if (s === "ok") return "ok";
  if (s === "refused") return "warn";
  if (s === "unreachable" || s.indexOf("error") === 0) return "bad";
  return "";
}
// the segments of a bar: only the non-zero counts, each {key, n}
function barParts(parts) {
  return (parts || []).filter(function (p) { return p && p.n > 0; });
}
// one of the events `names` the agent published: a watcher of an agent also sees every
// call made to it (a view's own reads among them), and those are never re-read on
function isEvent(ev, names) {
  return !!ev && typeof ev === "object" && typeof ev.type === "string" && (names || []).indexOf(ev.type) >= 0;
}
// end pure
function h(tag, cls, text) {
  var n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text != null) n.textContent = String(text);
  return n;
}
function put(parent) {
  for (var i = 1; i < arguments.length; i++) if (arguments[i]) parent.appendChild(arguments[i]);
  return parent;
}
function pillOf(text, cls) { return h("span", "pill" + (cls ? " " + cls : ""), text); }
function section(title, via, sub) {
  var s = h("div", "sec");
  var sh = put(h("div", "sh"), h("h2", null, title));
  if (sub) put(sh, h("span", "sub", sub));
  put(sh, h("span", "gap"), via ? h("span", "via", via) : null);
  put(s, sh);
  return s;
}
function kpi(label, value, note, cls) {
  var b = h("b", cls || null, value);
  return put(h("div", "kpi"), h("span", null, label), b, note ? h("small", null, note) : null);
}
function legend(parts) {
  var l = h("div", "leg");
  parts.forEach(function (p) {
    var u = h("u");
    u.style.background = p.color;
    put(l, put(h("span"), u, h("b", null, p.n), document.createTextNode(" " + p.label)));
  });
  return l;
}
function bar(parts) {
  var b = h("div", "bar");
  barParts(parts).forEach(function (p) {
    var i = h("i");
    i.style.flex = String(p.n);
    i.style.background = p.color;
    b.appendChild(i);
  });
  return b;
}
function call(target, p) { return fantastic.send(target, p).then(function (r) { return r; }, function () { return null; }); }
// follow the agent: `fn(ev)` on each event of `names` it publishes (the kernel's emit on its id),
// never on a timer. The view's `calls` name `watch` on its agent.
function follow(agent, names, fn) {
  return fantastic.watch(agent, function (ev) { if (isEvent(ev, names)) fn(ev); });
}
