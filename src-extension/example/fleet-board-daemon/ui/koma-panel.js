// koma-panel.js — copyable helper for talking to koma from an extension
// panel iframe. See docs/EXTENSIONS.md's "Panel bridge" section for the
// full envelope spec this implements; this file is deliberately dependency
// free so you can drop it straight into any panel's ui/ directory as-is.
//
// Envelope this sends (panel -> host), verbatim:
//   { koma: 'panel', v: 1, kind: 'msg', reqId: <string>, payload: <any> }
// posted to `window.parent` with targetOrigin '*'. The panel iframe and its
// host chrome are DIFFERENT origins by design (koma://extension/<id>/... vs
// koma://localhost/), so there is no meaningful same-origin target to pass
// instead — and the host never trusts this message's claimed identity
// anyway: it attributes the sender by which iframe `Window` the `message`
// event actually came from, never by anything inside the payload.
//
// Envelopes this listens for (host -> panel):
//   { koma: 'host', v: 1, kind: 'reply', reqId, ok, payload?, error? }
//   { koma: 'host', v: 1, kind: 'push', payload }
// A 'reply' resolves/rejects the promise `send()` returned for that reqId.
// A 'push' is unsolicited — sent by the daemon extension calling
// `Koma::panel_push` with no request behind it — and is fanned out to every
// handler registered with `onPush()`.

(function (global) {
  'use strict';

  var nextReqId = 1;
  var pending = new Map(); // reqId -> { resolve, reject, timer }
  var pushHandlers = [];

  /**
   * Send `payload` to the daemon extension backing this panel and resolve
   * with its reply payload (or reject with an Error on `ok:false`, a
   * malformed reply, or a timeout). `timeoutMs` defaults to 15000 — keep it
   * well under the host's own panel-invoke timeout so you see YOUR timeout
   * first, not a generic failure.
   */
  function send(payload, timeoutMs) {
    timeoutMs = timeoutMs || 15000;
    var reqId = String(nextReqId++);
    return new Promise(function (resolve, reject) {
      var timer = setTimeout(function () {
        pending.delete(reqId);
        reject(new Error('koma panel request timed out: ' + reqId));
      }, timeoutMs);
      pending.set(reqId, { resolve: resolve, reject: reject, timer: timer });
      window.parent.postMessage(
        { koma: 'panel', v: 1, kind: 'msg', reqId: reqId, payload: payload },
        '*'
      );
    });
  }

  /** Register a handler for unsolicited daemon->panel pushes. */
  function onPush(handler) {
    pushHandlers.push(handler);
  }

  window.addEventListener('message', function (event) {
    var data = event.data;
    if (!data || data.koma !== 'host' || data.v !== 1) return;

    if (data.kind === 'reply') {
      var entry = pending.get(data.reqId);
      if (!entry) return; // unknown/expired reqId (e.g. already timed out) — ignore
      pending.delete(data.reqId);
      clearTimeout(entry.timer);
      if (data.ok) {
        entry.resolve(data.payload);
      } else {
        entry.reject(new Error(data.error || 'koma panel request failed'));
      }
      return;
    }

    if (data.kind === 'push') {
      for (var i = 0; i < pushHandlers.length; i++) {
        pushHandlers[i](data.payload);
      }
    }
  });

  global.KomaPanel = { send: send, onPush: onPush };
})(window);
