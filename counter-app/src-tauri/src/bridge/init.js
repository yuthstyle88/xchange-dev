// window.xcCounter: the desk's one way to reach the app (docs/design/xc-counter-app.md §3, §8.6).
// Present only inside the app. This file is the contract; web/src/shared/xc-counter/bridge.ts
// adds bindCounter() on top of it.
(function () {
  'use strict';
  if (window !== window.top) return;

  var REASONS = ['noScanner', 'busy', 'timeout', 'coverOpen', 'device'];

  function invoke(command) {
    return window.__TAURI_INTERNALS__.invoke(command);
  }

  Object.defineProperty(window, 'xcCounter', {
    value: Object.freeze({
      // { agentId, counterName, branchId, capabilities: { cashDrawer, scanner, kioskLock }, appVersion, build }
      info: function () {
        return invoke('info');
      },
      // { ok: true, file: Blob(image/jpeg) } or { ok: false, reason }; it never rejects.
      scanPassport: function () {
        return invoke('scan_passport').then(
          function (bytes) {
            return { ok: true, file: new Blob([bytes], { type: 'image/jpeg' }) };
          },
          function (err) {
            return { ok: false, reason: REASONS.indexOf(err) >= 0 ? err : 'device' };
          }
        );
      },
      // The one-time nonce for deskBindCounter (§5.4), as a string.
      bindingNonce: function () {
        return invoke('desk_binding').then(function (binding) {
          return binding.nonce;
        });
      },
    }),
    enumerable: true,
  });
})();
