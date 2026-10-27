// What runs inside a card's iframe: `fantastic.send` and `fantastic.watch` that go to the
// page by postMessage, and the height the panel wants.
(function () {
  var seq = 0, pending = {}, watchers = {};
  window.addEventListener("message", function (e) {
    var m = e.data || {};
    if (m.t === "reply" && pending[m.n]) { var p = pending[m.n]; delete pending[m.n]; if (m.error) p.reject(new Error(m.error)); else p.resolve(m.reply); }
    else if (m.t === "event" && watchers[m.n]) { watchers[m.n](m.ev); }
  });
  window.fantastic = {
    send: function (id, payload) { return new Promise(function (resolve, reject) { var n = ++seq; pending[n] = { resolve: resolve, reject: reject }; parent.postMessage({ t: "send", n: n, id: id, payload: payload }, "*"); }); },
    watch: function (id, cb) { var n = ++seq; watchers[n] = cb; parent.postMessage({ t: "watch", n: n, id: id }, "*"); return function () { delete watchers[n]; parent.postMessage({ t: "unwatch", n: n }, "*"); }; }
  };
  function size() { parent.postMessage({ t: "size", h: document.documentElement.scrollHeight }, "*"); }
  window.addEventListener("load", size);
  if (window.ResizeObserver) new ResizeObserver(size).observe(document.documentElement);
})();
