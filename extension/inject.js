// reIDM Integration — MAIN-world hook.
//
// Runs in the *page's* JavaScript context, so it can see the requests the page itself makes.
// `chrome.webRequest` already reports every media URL, but it cannot read response bodies, and
// many players fetch a manifest whose URL carries no extension (or whose variants are listed
// only inside the body). This hook covers that gap:
//
//   * fetch(): report the request URL, and read a manifest body to learn its variant URLs.
//   * XMLHttpRequest: report the URL.
//   * MediaSource.addSourceBuffer: a player using MSE usually means a stream is playing.
//
// It reports by postMessage to the content script, which forwards to the service worker. It
// never blocks or modifies a request — the page must keep working exactly as before.

(() => {
  if (window.__reidmInjected) return;
  window.__reidmInjected = true;

  const REPORT = (items) => {
    if (!items.length) return;
    window.postMessage({ __reidm: true, items }, '*');
  };

  const MEDIA = /\.(m3u8|mpd|mp4|m4v|webm|mkv|mov|flv|ts|mp3|m4a|flac|ogg|opus|wav|aac)(\?|#|$)/i;
  const MANIFEST = /\.(m3u8|mpd)(\?|#|$)/i;
  // A manifest can also be served from a URL with no extension; the content type is the tell,
  // but for the request side we fall back to a few well-known path shapes.
  const LIKELY_MANIFEST = /(?:^|\/)(manifest|playlist|master|index)\.(m3u8|mpd)/i;

  /** Pull variant URLs out of an HLS or DASH manifest body. */
  function variantsFrom(body, baseUrl) {
    const out = [];
    // HLS: variant playlists are listed as URIs on their own lines; segments use #EXTINF.
    for (const line of body.split(/\r?\n/)) {
      const t = line.trim();
      if (!t || t.startsWith('#')) continue;
      if (/\.(m3u8|ts|m4s|mp4|aac|vtt)(\?|#|$)/i.test(t)) out.push(t);
    }
    // DASH: <BaseURL>…</BaseURL>.
    for (const m of body.matchAll(/<BaseURL>([^<]+)<\/BaseURL>/gi)) out.push(m[1]);
    return out
      .slice(0, 50)
      .map((u) => {
        try {
          return new URL(u, baseUrl).href;
        } catch {
          return null;
        }
      })
      .filter(Boolean);
  }

  const origFetch = window.fetch;
  if (typeof origFetch === 'function') {
    window.fetch = function (input, init) {
      let url = null;
      try {
        url = typeof input === 'string' ? input : input && input.url ? input.url : String(input);
      } catch {
        /* ignore */
      }
      const p = origFetch.apply(this, arguments);
      if (url && (MANIFEST.test(url) || LIKELY_MANIFEST.test(url))) {
        REPORT([{ url, kind: 'stream' }]);
        // Read a copy for the variant list; never disturb what the caller receives.
        p.then((r) => {
          try {
            r.clone()
              .text()
              .then((body) => {
                const kids = variantsFrom(body, url);
                if (kids.length) REPORT(kids.map((u) => ({ url: u, kind: 'segment' })));
              })
              .catch(() => {});
          } catch {
            /* a streamed or already-consumed body */
          }
        }).catch(() => {});
      } else if (url && MEDIA.test(url)) {
        REPORT([{ url, kind: 'media' }]);
      }
      return p;
    };
  }

  const OrigXHR = window.XMLHttpRequest;
  if (OrigXHR && OrigXHR.prototype && OrigXHR.prototype.open) {
    const open = OrigXHR.prototype.open;
    OrigXHR.prototype.open = function (method, url) {
      try {
        const abs = url ? new URL(url, document.baseURI).href : null;
        if (abs && (MANIFEST.test(abs) || MEDIA.test(abs) || LIKELY_MANIFEST.test(abs))) {
          REPORT([{ url: abs, kind: MANIFEST.test(abs) || LIKELY_MANIFEST.test(abs) ? 'stream' : 'media' }]);
        }
      } catch {
        /* ignore */
      }
      return open.apply(this, arguments);
    };
  }

  // MSE in use is a strong hint that a stream is playing even when no single URL looks like one.
  if (window.MediaSource && window.MediaSource.prototype && window.MediaSource.prototype.addSourceBuffer) {
    const add = window.MediaSource.prototype.addSourceBuffer;
    window.MediaSource.prototype.addSourceBuffer = function (mime) {
      try {
        REPORT([{ url: location.href, kind: 'mse', mime: String(mime) }]);
      } catch {
        /* ignore */
      }
      return add.apply(this, arguments);
    };
  }
})();
