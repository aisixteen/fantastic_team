// The dispatcher page outside the canvas has no `fantastic` object: this makes
// the two calls the page uses (`send` and `watch`) over the web_ws leg of the
// team that serves it. Frames: {type:'call', target, payload, id} answered by
// {type:'reply', id, data}; {type:'watch', src} answered by {type:'event', src,
// payload} frames. Nothing from the kernel is ever put in as markup.
(function () {
  var proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  var ws = new WebSocket(proto + '//' + location.host + '/team_ws/ws');
  var pending = [];
  var calls = {};
  var watchers = {};
  var next = 1;
  function out(frame) {
    var text = JSON.stringify(frame);
    if (ws.readyState === 1) ws.send(text); else pending.push(text);
  }
  ws.onopen = function () {
    pending.splice(0).forEach(function (t) { ws.send(t); });
  };
  ws.onclose = function () {
    Object.keys(calls).forEach(function (id) {
      var c = calls[id];
      delete calls[id];
      c.reject(new Error('the connection to the team closed'));
    });
  };
  ws.onmessage = function (m) {
    var f;
    try { f = JSON.parse(m.data); } catch (e) { return; }
    if (f.type === 'reply' || f.type === 'error') {
      var c = calls[f.id];
      if (!c) return;
      delete calls[f.id];
      if (f.type === 'reply') c.resolve(f.data);
      else c.reject(new Error(String(f.error || 'the call was refused')));
    } else if (f.type === 'event') {
      (watchers[f.src] || []).slice().forEach(function (cb) {
        try { cb(f.payload); } catch (e) { /* one listener never stops the rest */ }
      });
    }
  };
  window.fantastic = {
    send: function (target, payload) {
      return new Promise(function (resolve, reject) {
        var id = 'p' + next++;
        calls[id] = { resolve: resolve, reject: reject };
        out({ type: 'call', target: target, payload: payload, id: id });
      });
    },
    watch: function (src, cb) {
      var list = watchers[src] || (watchers[src] = []);
      if (list.length === 0) out({ type: 'watch', src: src });
      list.push(cb);
      return function () {
        var l = watchers[src] || [];
        var i = l.indexOf(cb);
        if (i >= 0) l.splice(i, 1);
        if (l.length === 0) out({ type: 'unwatch', src: src });
      };
    }
  };
})();
