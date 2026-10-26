// Firefox exposes `browser.*` with promises and `chrome.*` with callbacks. The shared extension
// code is written for Chrome's MV3 shape: some calls are awaited, some take a callback and read
// `runtime.lastError`. This shim wraps Firefox's `browser` so both styles work, under one name
// (`reidmApi`) that the Firefox build substitutes for `chrome`.
//
// Event objects (`runtime.onMessage`, `downloads.onCreated`, …) are returned untouched: wrapping
// them would break addListener's identity checks. Functions that return a value (getURL,
// getManifest, connect) keep returning it; promises are returned when no callback was given.

export const SHIM = String.raw`/* Firefox: Chrome-style callbacks and browser.* promises under one name. */
var reidmApi = globalThis.reidmApi || (globalThis.reidmApi = (() => {
  const b = globalThis.browser;
  if (!b) return globalThis.chrome;
  let lastError;
  const isEvent = (v) => v && typeof v === 'object' && typeof v.addListener === 'function' && typeof v.removeListener === 'function';
  const wrap = (ns) => new Proxy(ns, {
    get(t, k) {
      if (k === 'lastError') return lastError;
      const v = t[k];
      if (typeof v === 'function') {
        return function () {
          const args = Array.prototype.slice.call(arguments);
          const cb = typeof args[args.length - 1] === 'function' ? args.pop() : null;
          let r;
          try {
            r = v.apply(t, args);
          } catch (e) {
            if (cb) {
              lastError = e;
              try { cb(undefined); } finally { lastError = undefined; }
              return;
            }
            throw e;
          }
          if (r && typeof r.then === 'function') {
            if (cb) {
              r.then(
                function (x) { lastError = undefined; cb(x); },
                function (e) { lastError = e; try { cb(undefined); } finally { lastError = undefined; } }
              );
              return;
            }
            return r;
          }
          if (cb) { cb(r); return; }
          return r;
        };
      }
      if (v && typeof v === 'object' && !isEvent(v)) return wrap(v);
      return v;
    },
    set(t, k, v) { t[k] = v; return true; }
  });
  return wrap(b);
})());
`;

/** A tiny fake of the Firefox APIs the extension uses, to prove both call styles work. */
export function selfTest() {
  const fake = {
    runtime: {
      getURL: (p) => `moz-extension://x/${p}`,
      sendMessage: async (m) => ({ ok: true, got: m }),
      onMessage: { addListener() {}, removeListener() {} },
    },
    downloads: {
      pause: async (id) => id,
      onCreated: { addListener() {}, removeListener() {} },
    },
    storage: { local: { get: async () => ({ a: 1 }) } },
  };
  // The shim reads the real global `browser`, as it does in Firefox; stand one up for the test.
  const prevBrowser = globalThis.browser;
  const prevApi = globalThis.reidmApi;
  globalThis.browser = fake;
  delete globalThis.reidmApi;
  return (async () => {
    try {
      const api = new Function(`${SHIM}\nreturn reidmApi;`)();
      const out = [];
      const assert = (ok, what) => { if (!ok) throw new Error(`shim self-test: ${what}`); out.push(what); };
      assert(api.runtime.getURL('a.js') === 'moz-extension://x/a.js', 'sync getURL keeps its value');
      assert(typeof api.runtime.onMessage.addListener === 'function', 'events are passed through');
      assert((await api.storage.local.get()).a === 1, 'promises are returned when awaited');
      const sent = await api.runtime.sendMessage({ x: 1 });
      assert(sent.ok === true, 'promise results are kept');
      let seen;
      api.downloads.pause(7, (v) => { seen = v; });
      await new Promise((r) => setTimeout(r, 0));
      assert(seen === 7, 'a callback is called with the result');
      return out;
    } finally {
      globalThis.browser = prevBrowser;
      globalThis.reidmApi = prevApi;
    }
  })();
}
