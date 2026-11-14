// What runs inside a card's iframe: `fantastic.send` and `fantastic.watch` that go to the
// page by postMessage, `emit` (a message to the cell the view is in), `onMessage` (what the page gives
// back) and the height the panel wants.
(function () {
  var seq = 0, pending = {}, watchers = {}, inbox = [];
  window.addEventListener("message", function (e) {
    var m = e.data || {};
    if (m.t === "reply" && pending[m.n]) { var p = pending[m.n]; delete pending[m.n]; if (m.error) p.reject(new Error(m.error)); else p.resolve(m.reply); }
    else if (m.t === "event" && watchers[m.n]) { watchers[m.n](m.ev); }
    else if (m.t === "inbox") { inbox.slice().forEach(function (cb) { try { cb(m.payload); } catch (x) { /* one listener never stops the rest */ } }); }
  });
  window.fantastic = {
    send: function (id, payload) { return new Promise(function (resolve, reject) { var n = ++seq; pending[n] = { resolve: resolve, reject: reject }; parent.postMessage({ t: "send", n: n, id: id, payload: payload }, "*"); }); },
    emit: function (id, payload) { parent.postMessage({ t: "emit", id: id, payload: payload || {} }, "*"); },
    onMessage: function (cb) { inbox.push(cb); return function () { var i = inbox.indexOf(cb); if (i >= 0) inbox.splice(i, 1); }; },
    watch: function (id, cb) { var n = ++seq; watchers[n] = cb; parent.postMessage({ t: "watch", n: n, id: id }, "*"); return function () { delete watchers[n]; parent.postMessage({ t: "unwatch", n: n }, "*"); }; }
  };
  function size() { parent.postMessage({ t: "size", h: document.documentElement.scrollHeight }, "*"); }
  window.addEventListener("load", size);
  if (window.ResizeObserver) new ResizeObserver(size).observe(document.documentElement);
})();
